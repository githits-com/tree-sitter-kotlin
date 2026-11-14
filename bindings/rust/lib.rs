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
