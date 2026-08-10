/**
 * Tree-sitter grammar for BullScript.
 *
 * `Bullscript/src/lang/grammar` — the hand-written lexer and parser — stays the
 * authority. This grammar exists because Zed requires one for any language an
 * extension defines, and it drives highlighting and structure only.
 *
 * Deliberately more permissive than the real parser, for the same reason as
 * the Bullang grammar: tree-sitter runs on every keystroke, including on a
 * half-typed line, and a grammar that refused such input would drop
 * highlighting exactly when it is most useful. Errors come from the language
 * server, which runs BullScript's own parser.
 *
 * A .busc file is pipes and nothing else — no declarations, no directives, no
 * escape blocks.
 */

module.exports = grammar({
  name: "bullscript",

  extras: ($) => [/\s/, $.comment],

  rules: {
    source_file: ($) => repeat($.pipe),

    comment: (_) => token(seq("//", /[^\n]*/)),

    // (inputs) : value -> {binding: type};
    pipe: ($) =>
      seq($.input_list, ":", field("value", $._pipe_value), "->", $.binding, ";"),

    input_list: ($) =>
      seq("(", optional(seq($.input, repeat(seq(",", $.input)))), ")"),

    // Every slot carries its type. A named slot is a parameter; a literal slot
    // is a value the script already holds.
    input: ($) =>
      seq(field("name", choice($.identifier, $.number, $.string)), ":", field("type", $.type)),

    // The binding carries its type too, because BullScript has no separate
    // declaration to carry it.
    binding: ($) =>
      seq("{", field("name", $.identifier), ":", field("type", $.type), "}"),

    _pipe_value: ($) => choice($.builtin_call, $.bag_call, $._expression),

    builtin_call: ($) => seq("builtin", "::", field("name", $.identifier)),

    bag_call: ($) => seq("bag", "::", field("name", $.identifier)),

    _expression: ($) => choice($.binary_expr, $._atom),

    binary_expr: ($) =>
      seq(field("left", $._atom), field("operator", $.operator), field("right", $._atom)),

    operator: (_) =>
      choice("&&", "||", "==", "!=", "<=", ">=", "+", "-", "*", "/", "%", "<", ">"),

    _atom: ($) => choice($.number, $.string, $.boolean, $.identifier),

    type: (_) => choice("i64", "f64", "bool", "String"),

    boolean: (_) => choice("true", "false"),

    identifier: (_) => /[a-zA-Z_][a-zA-Z0-9_]*/,

    number: (_) => token(seq(optional("-"), /\d+/, optional(seq(".", /\d+/)))),

    string: (_) => token(seq('"', repeat(choice(/\\./, /[^"\\\n]/)), '"')),
  },
});
