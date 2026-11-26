#include "tree_sitter/parser.h"

#include <string.h>
#include <wctype.h>

enum TokenType {
    SEMI,
    CLASS_MEMBER_SEMI,
    BLOCK_COMMENT,
    NOT_IS,
    IN,
    Q_DOT,
    MULTILINE_STRING_CONTENT,
    CONSTRUCTOR,
    GET,
    SET,
    DOLLAR,
};

#define MAX_WORD_SIZE 16
#define MAX_WORDS 16

static inline void advance(TSLexer *lexer) { lexer->advance(lexer, false); }

static inline void skip(TSLexer *lexer) { lexer->advance(lexer, true); }

static bool scan_word(TSLexer *lexer, const char *const word) {
    for (uint8_t i = 0; word[i] != '\0'; i++) {
        if (lexer->lookahead != word[i]) {
            return false;
        }
        skip(lexer);
    }
    return true;
}

static bool scan_words(TSLexer *lexer, const char words[MAX_WORDS][MAX_WORD_SIZE], char scanned_word[16],
                       uint8_t *index) {
    if (!scanned_word[0]) {
        for (uint8_t i = 0; i < MAX_WORD_SIZE - 1; i++) {
            if (!iswalpha(lexer->lookahead)) {
                if (i == 0) {
                    return false;
                }
                break;
            }
            scanned_word[i] = (char)lexer->lookahead;
            skip(lexer);
        }
    }

    for (uint8_t i = 0; i < MAX_WORDS; i++) {
        if (strncmp(scanned_word, words[i], MAX_WORD_SIZE) == 0) {
            if (index != NULL) {
                *index = i;
            }
            return true;
        }
    }

    return false;
}

// Parser recovery accepts a block comment only when it closes within this
// nesting depth. No comment in the measured Kotlin corpora nests deeper than 1.
#define RECOVERY_COMMENT_MAX_DEPTH 8

// Walks the rest of a nested block comment after its opening `/*`. Returns
// false when the input ends first, or when the comment nests deeper than
// max_depth.
static bool finish_block_comment(TSLexer *lexer, bool skip_chars, unsigned max_depth) {
    bool after_star = false;
    unsigned nesting_depth = 1;
    while (!lexer->eof(lexer)) {
        switch (lexer->lookahead) {
            case '*':
                lexer->advance(lexer, skip_chars);
                after_star = true;
                break;
            case '/':
                lexer->advance(lexer, skip_chars);
                if (after_star) {
                    after_star = false;
                    nesting_depth--;
                    if (nesting_depth == 0) {
                        return true;
                    }
                } else if (lexer->lookahead == '*') {
                    if (++nesting_depth > max_depth) {
                        return false;
                    }
                    lexer->advance(lexer, skip_chars);
                }
                break;
            default:
                lexer->advance(lexer, skip_chars);
                after_star = false;
                break;
        }
    }
    return false;
}

// Kotlin block comments nest. Outside recovery an unterminated comment extends
// to EOF, as in the compiler, so a later scan never walks the same
// unterminated suffix again.
//
// During recovery the comment must close. Recovery can skip a string's opening
// quote and read the string's interior as code, where a `/*` such as the one in
// "image/*" would open a comment that hides every later declaration. Each
// opener nested inside a walk raises its depth, so the depth limit stops every
// walk after a bounded number of later unclosed openers. Declined recovery
// openers therefore cost time linear in the file.
static bool scan_block_comment(TSLexer *lexer, bool in_recovery) {
    if (lexer->lookahead != '/') {
        return false;
    }
    advance(lexer);
    if (lexer->lookahead != '*') {
        return false;
    }
    advance(lexer);
    bool closed = finish_block_comment(lexer, false, in_recovery ? RECOVERY_COMMENT_MAX_DEPTH : UINT32_MAX);
    if (!closed && in_recovery) {
        return false;
    }
    lexer->result_symbol = BLOCK_COMMENT;
    lexer->mark_end(lexer);
    return true;
}

static bool skip_quoted_on_line(TSLexer *lexer, int32_t delimiter);

static inline bool is_identifier_char(int32_t c) { return iswalnum(c) || c == '_'; }

// Skips a string literal whose first quote is the lookahead. A raw string runs
// to the next `"""`; any other string ends at its closing quote or at the end
// of its line, because ordinary strings cannot span lines. Templates are not
// parsed, so a quote inside `${...}` can end the skipped region early, but
// only within that line. Returns false when the input ends first.
static bool skip_string_literal(TSLexer *lexer) {
    skip(lexer);
    if (lexer->lookahead == '"') {
        skip(lexer);
        if (lexer->lookahead != '"') {
            return true;
        }
        skip(lexer);
        unsigned quotes = 0;
        while (!lexer->eof(lexer)) {
            if (lexer->lookahead == '"') {
                quotes++;
            } else if (quotes >= 3) {
                return true;
            } else {
                quotes = 0;
            }
            skip(lexer);
        }
        return quotes >= 3;
    }
    return skip_quoted_on_line(lexer, '"');
}

// Skips a character literal or backticked identifier whose opening delimiter
// is the lookahead, stopping at its closing delimiter or the end of its line.
static bool skip_quoted_on_line(TSLexer *lexer, int32_t delimiter) {
    while (!lexer->eof(lexer)) {
        int32_t c = lexer->lookahead;
        if (c == '\n') {
            return true;
        }
        skip(lexer);
        if (c == '\\' && delimiter != '`') {
            if (lexer->eof(lexer)) {
                return false;
            }
            skip(lexer);
        } else if (c == delimiter) {
            return true;
        }
    }
    return false;
}

// Decides whether `set(` on a new line after a property is a call statement,
// which needs an automatic semicolon before it, or a setter. The lookahead is
// the opening parenthesis.
//
// Only local properties are followed by `set(...)` calls; members and
// top-level properties always take a setter there. As before, a call is
// recognized by a line break right after its matching `)`, and an unclosed list
// keeps the setter reading. The scan stops at the first `{`: a setter
// parameter list never contains a block or lambda, so the text is a call. It
// stops at an unmatched `}` or a `val`/`var` keyword, which cannot occur in
// either list and mark the list as unclosed. Every other property begins with
// `val` or `var`, and a property nested inside an earlier one ends before a
// `}`, so no scan reaches past the next property's start. Unclosed setters
// therefore cost time linear in the file instead of a walk to EOF each.
static bool set_call_follows(TSLexer *lexer) {
    unsigned depth = 0;
    bool in_word = false;
    while (!lexer->eof(lexer)) {
        int32_t c = lexer->lookahead;
        if (is_identifier_char(c)) {
            if (in_word || !iswalpha(c)) {
                skip(lexer);
                in_word = true;
                continue;
            }
            char word[4] = {0};
            uint8_t length = 0;
            while (is_identifier_char(lexer->lookahead)) {
                if (length < sizeof(word)) {
                    word[length] = (char)lexer->lookahead;
                }
                length++;
                skip(lexer);
            }
            if (length == 3 && (memcmp(word, "val", 3) == 0 || memcmp(word, "var", 3) == 0)) {
                return false;
            }
            in_word = false;
            continue;
        }
        in_word = false;
        switch (c) {
            case '(':
                depth++;
                skip(lexer);
                break;
            case ')':
                skip(lexer);
                if (--depth == 0) {
                    while (iswspace(lexer->lookahead)) {
                        if (lexer->lookahead == '\n') {
                            return true;
                        }
                        skip(lexer);
                    }
                    // A trailing line comment also ends the line.
                    if (lexer->lookahead == '/') {
                        skip(lexer);
                        return lexer->lookahead == '/';
                    }
                    return false;
                }
                break;
            case '{':
                return true;
            case '}':
                return false;
            case '"':
                if (!skip_string_literal(lexer)) {
                    return false;
                }
                break;
            case '\'':
            case '`':
                skip(lexer);
                if (!skip_quoted_on_line(lexer, c)) {
                    return false;
                }
                break;
            case '/':
                skip(lexer);
                if (lexer->lookahead == '/') {
                    while (!lexer->eof(lexer) && lexer->lookahead != '\n') {
                        skip(lexer);
                    }
                } else if (lexer->lookahead == '*') {
                    skip(lexer);
                    if (!finish_block_comment(lexer, true, UINT32_MAX)) {
                        return false;
                    }
                }
                break;
            default:
                skip(lexer);
                break;
        }
    }
    return false;
}

void *tree_sitter_kotlin_external_scanner_create() { return NULL; }

void tree_sitter_kotlin_external_scanner_destroy(void *payload) {}

unsigned tree_sitter_kotlin_external_scanner_serialize(void *payload, char *buffer) { return 0; }

void tree_sitter_kotlin_external_scanner_deserialize(void *payload, const char *buffer, unsigned length) {}

bool tree_sitter_kotlin_external_scanner_scan(void *payload, TSLexer *lexer, const bool *valid_symbols) {
    // Only parser recovery enables both automatic semicolon types at once.
    // Recovery still needs block comments, which are extras in every state.
    // Leave all other recovery input to the internal lexer instead of
    // repeatedly scanning unclassified source as multiline string content.
    if (valid_symbols[SEMI] && valid_symbols[CLASS_MEMBER_SEMI]) {
        while (iswspace(lexer->lookahead)) {
            skip(lexer);
        }
        return scan_block_comment(lexer, true);
    }

    if (valid_symbols[MULTILINE_STRING_CONTENT]) {
        bool did_advance = false;
        lexer->result_symbol = MULTILINE_STRING_CONTENT;
        while (!lexer->eof(lexer)) {
            switch (lexer->lookahead) {
                case '$':
                    lexer->mark_end(lexer);
                    advance(lexer);
                    if (iswalpha(lexer->lookahead) || lexer->lookahead == '{') {
                        return did_advance;
                    }
                    did_advance = true;
                    break;
                case '"':
                    lexer->mark_end(lexer);
                    // 3 or 4 quotes means we're done
                    advance(lexer);
                    if (lexer->lookahead == '"') {
                        advance(lexer);
                        if (lexer->lookahead == '"') {
                            advance(lexer);
                            if (lexer->lookahead == '"') {
                                advance(lexer);
                            }
                            return did_advance;
                        }
                    }
                    did_advance = true;
                    break;
                default:
                    advance(lexer);
                    did_advance = true;
                    break;
            }
        }
    }

    if (valid_symbols[SEMI] || valid_symbols[CLASS_MEMBER_SEMI]) {
        lexer->result_symbol = valid_symbols[SEMI] ? SEMI : CLASS_MEMBER_SEMI;
        lexer->mark_end(lexer);
        bool saw_newline = false;
        for (;;) {
            if (lexer->eof(lexer)) {
                return true;
            }

            if (lexer->lookahead == ';') {
                advance(lexer);
                lexer->mark_end(lexer);
                return true;
            }

            if (!iswspace(lexer->lookahead)) {
                break;
            }

            if (lexer->lookahead == '\n') {
                skip(lexer);
                saw_newline = true;
                break;
            }

            if (lexer->lookahead == '\r') {
                skip(lexer);

                if (lexer->lookahead == '\n') {
                    skip(lexer);
                }

                saw_newline = true;
                break;
            }

            skip(lexer);
        }

        // Skip whitespace and comments
        while (iswspace(lexer->lookahead)) {
            skip(lexer);
        }
        if (lexer->lookahead == '/') {
            goto comment;
        }

        if (!saw_newline) {
            switch (lexer->lookahead) {
                case '!':
                    skip(lexer);
                    goto continue_not_is_from_semi;
                case '?':
                    if (valid_symbols[Q_DOT]) {
                        goto q_dot_from_semi;
                    }
                    return false;
                case 'i':
                    return scan_word(lexer, "import");
                case ';':
                    advance(lexer);
                    lexer->mark_end(lexer);
                    return true;
                default:
                    return false;
            }
        }

        char scanned_word[16] = {0};
    _switch:
        switch (lexer->lookahead) {
            case ',':
            case '.':
            case ':':
            case '*':
            case '%':
            case '>':
            case '<':
            case '=':
            case '{':
            case '[':
            case '|':
            case '&':
            case '/':
                return false;
            // Insert a semicolon before `--` and `++`, but not before binary `+` or `-`.
            // Insert before +/-{float}
            case '+':
                skip(lexer);
                if (lexer->lookahead == '+') {
                    return true;
                }
                return iswdigit(lexer->lookahead);
            case '-':
                skip(lexer);
                if (lexer->lookahead == '-') {
                    return true;
                }
                return iswdigit(lexer->lookahead);
            // Don't insert a semicolon before `!=`, but do insert one before a unary `!`.
            case '!':
                skip(lexer);
                if (lexer->lookahead == 'i' && valid_symbols[NOT_IS]) {
                    skip(lexer);
                    if (lexer->lookahead == 's') {
                        skip(lexer);
                        if (!iswalnum(lexer->lookahead)) {
                            return true;
                        }
                    }
                }
                return lexer->lookahead != '=';
            case '?':
                if (valid_symbols[Q_DOT]) {
                    goto q_dot_from_semi;
                }
                return true;
            case 'e':
            case 'i':
            case 'g':
            case 's':
            case 'p':
            case 'a':
            case 'f':
            case 'o':
            case 'l':
            case 'v':
            case 'n':
            case 'c':
            case 'b':
            case 'w':
                while (scan_words(lexer,
                                  (const char[16][16]){"public", "private", "protected", "internal", "abstract",
                                                       "final", "open", "override", "lateinit", "vararg", "noinline",
                                                       "crossinline", "external", "suspend", "inline"},
                                  scanned_word, NULL)) {
                    memset(scanned_word, 0, MAX_WORD_SIZE);
                    while (iswspace(lexer->lookahead)) {
                        skip(lexer);
                    }
                }

                uint8_t index = -1;
                bool res = scan_words(
                    lexer,
                    (const char[16][16]){"else", "in", "instanceof", "get", "set", "constructor", "by", "as", "where"},
                    scanned_word, &index);

                // If `CLASS_MEMBER_SEMI` is valid, we found a secondary constructor and so we want to insert a semi, OR
                // we found a variable named constructor whose field is being accessed
                if (index == 5) {
                    while (iswspace(lexer->lookahead)) {
                        skip(lexer);
                    }
                    if (valid_symbols[CLASS_MEMBER_SEMI] || lexer->lookahead == '.' || lexer->lookahead == '=') {
                        return true;
                    }
                }
                // Ordinarily, we should not insert a semicolon if there is an `else` on the next line,
                // except for when it's a 'when entry', which has a `->` after the `else`.
                else if (index == 0) {
                    while (iswspace(lexer->lookahead)) {
                        skip(lexer);
                    }
                    if (lexer->lookahead == '-') {
                        skip(lexer);
                        if (lexer->lookahead == '>') {
                            return true;
                        }
                    }
                }
                // If `get` was found and the keyword is not valid, return a semi since it's being used as an identifier
                else if (index == 3 && (!valid_symbols[GET] || lexer->lookahead == '[')) {
                    return true;
                }
                // If `set` was found and the keyword is not valid, return a semi since it's being used as an identifier
                else if (index == 4 && (!valid_symbols[SET] || lexer->lookahead == '[' || lexer->lookahead == '(' ||
                                        lexer->lookahead == '.')) {
                    if (lexer->lookahead == '(' && valid_symbols[SET]) {
                        return set_call_follows(lexer);
                    }
                    return true;
                }
                // If `in` was found and this specific external keyword is valid,
                // return a semi since it's being used in a range test
                else if (index == 1 && valid_symbols[IN]) {
                    return true;
                }
                return !res;
            case ';':
                advance(lexer);
                lexer->mark_end(lexer);
                return true;
            case '@':
                if (valid_symbols[CONSTRUCTOR]) {
                    while (!lexer->eof(lexer) && !iswspace(lexer->lookahead)) {
                        skip(lexer);
                    }
                    while (iswspace(lexer->lookahead)) {
                        skip(lexer);
                    }
                    char ctor[12] = "constructor";
                    for (uint8_t i = 0; i < 11; i++) {
                        if (lexer->lookahead != ctor[i]) {
                            return true;
                        }
                        skip(lexer);
                    }
                    return false;
                }
                if (valid_symbols[GET] || valid_symbols[SET]) {
                    bool saw_paren = false;
                    while (!lexer->eof(lexer) && (saw_paren ? lexer->lookahead != '\n' : !iswspace(lexer->lookahead))) {
                        skip(lexer);
                        if (lexer->lookahead == '(') {
                            saw_paren = true;
                        }
                        if (lexer->lookahead == ')') {
                            saw_paren = false;
                        }
                    }
                    while (iswspace(lexer->lookahead)) {
                        skip(lexer);
                    }
                    if (lexer->lookahead == '/') {
                        return true;
                    }
                    goto _switch;
                }
                return true;

            default:
                return true;
        }
    }

    while (iswspace(lexer->lookahead)) {
        skip(lexer);
    }

    if (valid_symbols[NOT_IS]) {
        if (lexer->lookahead == '!') {
            advance(lexer);
        continue_not_is_from_semi:
            if (lexer->lookahead == 'i') {
                advance(lexer);
                if (lexer->lookahead == 's') {
                    advance(lexer);
                    lexer->result_symbol = NOT_IS;
                    lexer->mark_end(lexer);
                    return !iswalnum(lexer->lookahead);
                }
            }
        }
    }

    if (valid_symbols[IN]) {
        if (lexer->lookahead == 'i') {
            advance(lexer);
            if (lexer->lookahead == 'n') {
                advance(lexer);
                lexer->result_symbol = IN;
                lexer->mark_end(lexer);
                return !iswalnum(lexer->lookahead);
            }
        }
    }

q_dot_from_semi:
    if (valid_symbols[Q_DOT]) {
        while (iswspace(lexer->lookahead)) {
            skip(lexer);
        }
        if (lexer->lookahead == '?') {
            advance(lexer);
            while (iswspace(lexer->lookahead)) {
                skip(lexer);
            }
            if (lexer->lookahead == '.') {
                advance(lexer);
                lexer->result_symbol = Q_DOT;
                lexer->mark_end(lexer);
                return true;
            }
        }
    }

comment:
    if (valid_symbols[DOLLAR]) {
        return false;
    }

    return scan_block_comment(lexer, false);
}
