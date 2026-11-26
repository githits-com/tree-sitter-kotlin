//! This crate provides Kotlin language support for the [tree-sitter][] parsing library.
//!
//! Typically, you will use the [LANGUAGE][] constant to add this language to a
//! tree-sitter [Parser][], and then use the parser to parse some code:
//!
//! ```
//! let code = r#"
//! // write some kotlin code
//! fun main(args : Array<String>) {
//!   println("Hello, World!")
//! }
//! "#;
//! let mut parser = tree_sitter::Parser::new();
//! let language = tree_sitter_kotlin_ng::LANGUAGE;
//! parser
//!     .set_language(&language.into())
//!     .expect("Error loading Kotlin parser");
//! let tree = parser.parse(code, None).unwrap();
//! assert!(!tree.root_node().has_error());
//! ```
//!
//! [Parser]: https://docs.rs/tree-sitter/*/tree_sitter/struct.Parser.html
//! [tree-sitter]: https://tree-sitter.github.io/

use tree_sitter_language::LanguageFn;

extern "C" {
    fn tree_sitter_kotlin() -> *const ();
}

/// The tree-sitter [`LanguageFn`][LanguageFn] for this grammar.
///
/// [LanguageFn]: https://docs.rs/tree-sitter-language/*/tree_sitter_language/struct.LanguageFn.html
pub const LANGUAGE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_kotlin) };

/// The content of the [`node-types.json`][] file for this grammar.
///
/// [`node-types.json`]: https://tree-sitter.github.io/tree-sitter/using-parsers#static-node-types
pub const NODE_TYPES: &str = include_str!("../../src/node-types.json");

#[cfg(test)]
mod tests {
    use std::ffi::{c_char, c_void};

    // Match the scanner callback layout in src/tree_sitter/parser.h.
    #[repr(C)]
    struct TestLexer {
        lookahead: i32,
        result_symbol: u16,
        advance: unsafe extern "C" fn(*mut Self, bool),
        mark_end: extern "C" fn(*mut Self),
        get_column: extern "C" fn(*mut Self) -> u32,
        is_at_included_range_start: extern "C" fn(*const Self) -> bool,
        eof: unsafe extern "C" fn(*const Self) -> bool,
        log: Option<unsafe extern "C" fn(*const Self, *const c_char, ...)>,
    }

    #[test]
    fn recovery_leaves_input_for_the_internal_lexer() {
        extern "C" {
            fn tree_sitter_kotlin_external_scanner_scan(
                payload: *mut c_void,
                lexer: *mut TestLexer,
                valid: *const bool,
            ) -> bool;
        }

        unsafe extern "C" fn advance(lexer: *mut TestLexer, _skip: bool) {
            // The scanner receives a writable pointer to the test-owned lexer.
            unsafe { (*lexer).lookahead = 0 };
        }
        unsafe extern "C" fn eof(lexer: *const TestLexer) -> bool {
            // The lexer is live for the entire scanner call.
            unsafe { (*lexer).lookahead == 0 }
        }

        let mut lexer = TestLexer {
            lookahead: i32::from(b'x'),
            result_symbol: 0,
            advance,
            mark_end: ignore_end,
            get_column: column,
            is_at_included_range_start: range_start,
            eof,
            log: None,
        };
        let recovery_symbols = [true; 11];
        // The state and lexer match the generated recovery state and C ABI.
        let recognized = unsafe {
            tree_sitter_kotlin_external_scanner_scan(
                std::ptr::null_mut(),
                &mut lexer,
                recovery_symbols.as_ptr(),
            )
        };
        assert!(!recognized);
        assert_eq!(lexer.lookahead, i32::from(b'x'));
    }

    #[test]
    fn recovery_still_recognizes_block_comments() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&super::LANGUAGE.into()).unwrap();
        let source = ")\n/** doc */\nfun recovered() = 1\n";
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();
        let sexp = root.to_sexp();

        assert!(root.has_error(), "{sexp}");
        assert!(sexp.contains("(block_comment)"), "{sexp}");
        let function = (0..root.named_child_count())
            .filter_map(|index| root.named_child(index))
            .find(|node| node.kind() == "function_declaration")
            .unwrap_or_else(|| panic!("{sexp}"));
        let name = function.child_by_field_name("name").unwrap();
        assert_eq!(&source[name.byte_range()], "recovered");
    }

    #[test]
    fn unterminated_block_comment_extends_to_eof() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&super::LANGUAGE.into()).unwrap();
        let source = "fun kept() = 1\n/* open /* nested */\nfun hidden() = 2\n";
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();
        let sexp = root.to_sexp();

        assert_eq!(root.named_child_count(), 2, "{sexp}");
        assert_eq!(root.named_child(0).unwrap().kind(), "function_declaration");
        let comment = root.named_child(1).unwrap();
        assert_eq!(comment.kind(), "block_comment", "{sexp}");
        assert_eq!(comment.end_byte(), source.len());
    }

    #[test]
    fn recovery_keeps_comment_openers_inside_strings() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&super::LANGUAGE.into()).unwrap();
        // Java syntax in a Kotlin file. One recovery path skips the opening
        // quote and would read `/*` inside the string as a comment to EOF.
        for source in [
            "try {} catch (E e) { \"/*\" }\nfun keep() {}\n",
            "try {} catch (E e) { \"/*\" }\n\nfun other() {}\n\nfun keep() {}\n",
        ] {
            let tree = parser.parse(source, None).unwrap();
            let root = tree.root_node();
            let sexp = root.to_sexp();
            assert!(!sexp.contains("block_comment"), "{source}: {sexp}");
            let kept = (0..root.named_child_count())
                .filter_map(|index| root.named_child(index))
                .any(|node| {
                    node.kind() == "function_declaration"
                        && node
                            .child_by_field_name("name")
                            .is_some_and(|name| &source[name.byte_range()] == "keep")
                });
            assert!(kept, "{source}: {sexp}");
        }
    }

    #[test]
    fn comment_opener_inside_string_stays_string_content() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&super::LANGUAGE.into()).unwrap();
        let source = "val pattern = \"/*\"\nfun x() = 1\n";
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();
        let sexp = root.to_sexp();

        assert!(!root.has_error(), "{sexp}");
        assert!(!sexp.contains("block_comment"), "{sexp}");
        assert_eq!(root.named_child_count(), 2, "{sexp}");
        assert_eq!(root.named_child(1).unwrap().kind(), "function_declaration");
    }

    // A scanner lexer over a byte buffer that counts every advance.
    #[repr(C)]
    struct BufferLexer {
        base: TestLexer,
        bytes: *const u8,
        len: usize,
        pos: usize,
        advances: usize,
    }

    unsafe extern "C" fn buffer_advance(lexer: *mut TestLexer, _skip: bool) {
        // `base` is the first field of the repr(C) BufferLexer that owns it.
        let lexer = unsafe { &mut *lexer.cast::<BufferLexer>() };
        lexer.advances += 1;
        if lexer.pos < lexer.len {
            lexer.pos += 1;
        }
        lexer.base.lookahead = if lexer.pos < lexer.len {
            // pos is in bounds of the borrowed buffer.
            i32::from(unsafe { *lexer.bytes.add(lexer.pos) })
        } else {
            0
        };
    }

    unsafe extern "C" fn buffer_eof(lexer: *const TestLexer) -> bool {
        // `base` is the first field of the repr(C) BufferLexer that owns it.
        let lexer = unsafe { &*lexer.cast::<BufferLexer>() };
        lexer.pos >= lexer.len
    }

    /// Runs one automatic-semicolon scan after a property, where SET is valid,
    /// and returns (inserted semicolon, characters examined).
    fn scan_after_property(source: &[u8]) -> (bool, usize) {
        // SEMI, GET and SET are valid after a property declaration.
        let mut valid = [false; 11];
        valid[0] = true;
        valid[8] = true;
        valid[9] = true;
        scan_buffer(source, &valid)
    }

    /// Runs one external scan over `source` and returns (recognized, characters
    /// examined).
    fn scan_buffer(source: &[u8], valid: &[bool; 11]) -> (bool, usize) {
        extern "C" {
            fn tree_sitter_kotlin_external_scanner_scan(
                payload: *mut c_void,
                lexer: *mut TestLexer,
                valid: *const bool,
            ) -> bool;
        }
        let mut lexer = BufferLexer {
            base: TestLexer {
                lookahead: source.first().map_or(0, |byte| i32::from(*byte)),
                result_symbol: 0,
                advance: buffer_advance,
                mark_end: ignore_end,
                get_column: column,
                is_at_included_range_start: range_start,
                eof: buffer_eof,
                log: None,
            },
            bytes: source.as_ptr(),
            len: source.len(),
            pos: 0,
            advances: 0,
        };
        // The BufferLexer outlives the call and starts with the TSLexer layout.
        let recognized = unsafe {
            tree_sitter_kotlin_external_scanner_scan(
                std::ptr::null_mut(),
                (&raw mut lexer).cast::<TestLexer>(),
                valid.as_ptr(),
            )
        };
        (recognized, lexer.advances)
    }

    #[test]
    fn unclosed_setter_parameters_stop_at_the_next_property() {
        let tail = "\nvar q: Int = 1\n    set(value\n".repeat(1_000);
        let source = format!("\n    set(value{tail}");
        let (inserted, advances) = scan_after_property(source.as_bytes());
        assert!(!inserted);
        assert!(
            advances < 40,
            "examined {advances} of {} bytes",
            source.len()
        );
    }

    #[test]
    fn recovery_declines_unclosed_comments_within_the_nesting_limit() {
        let recovery = [true; 11];
        let closed = b"/* outer /* inner */ */ fun f() {}";
        assert_eq!(scan_buffer(closed, &recovery), (true, 23));

        let unclosed = format!("/*\"){}", " /*\")".repeat(1_000));
        let (recognized, advances) = scan_buffer(unclosed.as_bytes(), &recovery);
        assert!(!recognized);
        assert!(
            advances < 60,
            "examined {advances} of {} bytes",
            unclosed.len()
        );
    }

    #[test]
    fn setter_scan_matches_parentheses_and_skips_literals() {
        for (source, inserted) in [
            ("\nset(value) {}\n", false),
            ("\nset(value: (Int) -> Unit) = Unit\n", false),
            ("\nset(@Suppress(\"x\") value) {}\n", false),
            ("\nset(value)\n", true),
            ("\nset(a, g(b))\n", true),
            ("\nset(\")\", \"}\", \"val\", '(')\n", true),
            ("\nset(\"\"\"\n}\nval\n\"\"\")\n", true),
            ("\nset(a /* ) } */, b) // )\n", true),
            ("\nset(foo(a) { b })\n", true),
            ("\nset(items.map { it })\n", true),
            ("\nset(@A(\"${\"{\"}\") value) {}\n", false),
            ("\nset(@A(\"${f(\")\")}\") value) {}\n", false),
            ("\nset(\"x\\\n)\n", true),
            ("\nset('\\\n)\n", true),
        ] {
            let (actual, _) = scan_after_property(source.as_bytes());
            assert_eq!(actual, inserted, "{source:?}");
        }
    }

    #[test]
    fn setter_scan_keeps_long_identifiers_distinct_from_val() {
        let source = format!("\nset({}val)\n", "x".repeat(256));
        assert_eq!(scan_after_property(source.as_bytes()).0, true);
    }

    #[test]
    fn annotated_setters_with_template_quotes_parse_without_recovery() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&super::LANGUAGE.into()).unwrap();
        let source =
            "class C {\n    var p = 0\n        set(@A(\"${\"{\"}\") value) { field = value }\n}\n";
        let tree = parser.parse(source, None).unwrap();
        let sexp = tree.root_node().to_sexp();
        assert!(!tree.root_node().has_error(), "{sexp}");
        assert!(sexp.contains("(setter"), "{sexp}");
    }

    #[test]
    fn escaped_line_breaks_do_not_continue_strings() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&super::LANGUAGE.into()).unwrap();
        for source in [
            "val a = \"x\\\nfun keep() {}\n",
            "val a = \"x\\\r\nfun keep() {}\r\n",
        ] {
            let tree = parser.parse(source, None).unwrap();
            let sexp = tree.root_node().to_sexp();
            assert!(sexp.contains("(function_declaration"), "{source:?}: {sexp}");
        }
    }

    #[test]
    fn local_set_calls_after_properties_stay_calls() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&super::LANGUAGE.into()).unwrap();
        for source in [
            "fun f() {\n    val x = 1\n    set(a, g(b))\n}\n",
            "fun f() {\n    val x = 1\n    set(\"}\")\n}\n",
            "fun f() {\n    val x = 1\n    set(foo(a) { b })\n}\n",
        ] {
            let tree = parser.parse(source, None).unwrap();
            let sexp = tree.root_node().to_sexp();
            assert!(!tree.root_node().has_error(), "{source}: {sexp}");
            assert!(!sexp.contains("(setter"), "{source}: {sexp}");
            assert!(sexp.contains("(call_expression"), "{source}: {sexp}");
        }
    }

    extern "C" fn ignore_end(_lexer: *mut TestLexer) {}
    extern "C" fn column(_lexer: *mut TestLexer) -> u32 {
        0
    }
    extern "C" fn range_start(_lexer: *const TestLexer) -> bool {
        false
    }

    #[test]
    fn test_can_load_grammar() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&super::LANGUAGE.into())
            .expect("Error loading Kotlin parser");
    }
    #[test]
    fn soft_keywords_remain_identifiers_outside_keyword_contexts() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&super::LANGUAGE.into()).unwrap();
        for keyword in [
            "abstract",
            "actual",
            "annotation",
            "by",
            "catch",
            "companion",
            "const",
            "constructor",
            "context",
            "crossinline",
            "data",
            "delegate",
            "dynamic",
            "enum",
            "expect",
            "external",
            "field",
            "file",
            "final",
            "finally",
            "get",
            "import",
            "infix",
            "init",
            "inline",
            "inner",
            "internal",
            "lateinit",
            "noinline",
            "open",
            "operator",
            "out",
            "override",
            "param",
            "private",
            "property",
            "protected",
            "public",
            "receiver",
            "reified",
            "sealed",
            "set",
            "setparam",
            "suspend",
            "tailrec",
            "value",
            "vararg",
            "where",
        ] {
            for source in [
                format!("{keyword}(\"x\")\n"),
                format!("fun sample() {{ row {{ {keyword}(\"x\") }}; text(\"${{value}}\") }}"),
                format!("fun sample({keyword}: String) = {keyword}"),
                format!("fun sample() {{ val {keyword} = \"x\"; row {{ {keyword}(\"x\") }} }}"),
                format!("fun sample() {{ row({keyword} = \"x\") }}"),
                format!("fun sample() {{ owner.{keyword}(\"x\") }}"),
                format!("class C<{keyword}>"),
                format!("fun <{keyword}> sample(value: {keyword}) = value"),
                format!("fun sample() {{ consume<{keyword}>() }}"),
                format!("class {keyword}"),
                format!("enum class C {{ {keyword} }}"),
                format!("var value: String = \"x\"\n set({keyword}) {{}}"),
                format!("inline fun <reified {keyword}> sample(value: {keyword}) = value"),
            ] {
                let tree = parser.parse(&source, None).unwrap();
                assert!(
                    !tree.root_node().has_error(),
                    "{source}: {}",
                    tree.root_node().to_sexp()
                );
            }
        }
    }

    #[test]
    fn annotations_preserve_modifier_declarations() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&super::LANGUAGE.into()).unwrap();
        for source in [
            "@Marker\npublic object Options {\npublic const val NAME: String = \"x\"\n}\n",
            "@Marker\nannotation class Marker",
            "@Marker\ndata class Value(val value: Int)",
            "class Outer {\n@Marker\ninner class Inner\n}\n",
            "inline fun <reified T> sample(crossinline crossinline: () -> Unit, noinline noinline: () -> Unit, vararg vararg: Int) {}",
        ] {
            let tree = parser.parse(source, None).unwrap();
            assert!(!tree.root_node().has_error(), "{source}: {}", tree.root_node().to_sexp());
            let kind = tree.root_node().named_child(0).unwrap().kind();
            assert!(matches!(kind, "class_declaration" | "object_declaration" | "function_declaration"), "{source}: {kind}");
        }
    }

    #[test]
    fn keyword_names_retain_generic_enum_and_setter_roles() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&super::LANGUAGE.into()).unwrap();
        for (source, expected_kind) in [
            ("class C<out, reified>", "type_parameter"),
            (
                "inline fun <reified out> sample(value: out) = value",
                "type_parameter",
            ),
            ("enum class C { open, companion, init }", "enum_entry"),
            ("var value: String = \"x\"\n set(crossinline) {}", "setter"),
        ] {
            let tree = parser.parse(source, None).unwrap();
            let sexp = tree.root_node().to_sexp();
            assert!(!tree.root_node().has_error(), "{source}: {sexp}");
            assert!(
                sexp.contains(&format!("({expected_kind}")),
                "{source}: {sexp}"
            );
        }
    }

    #[test]
    fn nested_open_callbacks_with_interpolation_parse_without_recovery() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&super::LANGUAGE.into()).unwrap();
        let source = r###"
fun x() {
    x(null) {
      x(x = x.x, x = x.x(x(1.5f))) {
        x(
          x(
            x("x", "x", "x", x = !x) { open(x.x) },
          ),
        )
      }
      x(x = x.x(x(1f)), x = x.x(x(0.5f))) {
        x(x(x.x), if (x.x) x.x else x.x)
        if (x.x) x("x", x.x)
        if (x.x) x("x", x.x)
        if (x.x == "x") x("x")
        (x.x.x { x(x.x) }).x { x(x(x)) }
        if (x.x != null) {
          x("${x}" + if (x.x == x.x) "x" else "x", x.x)
          if (x.x) x("x", x.x)
        }
      }
      x(x = x.x(x(1f)), x = x.x(x(1f))) {
        when {
          x && x.x != null -> {
            if (x.x && x != null) {
              x(x = { open(x.x(x.x)) }, x = x && !x.x, x = x.x("x")) {
                x("${x}")
              }
            }
          }
        }
      }
    }

}
"###;
        let tree = parser.parse(source, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "{}",
            tree.root_node().to_sexp()
        );
    }
}
