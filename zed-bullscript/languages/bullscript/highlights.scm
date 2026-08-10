; BullScript syntax highlighting.
;
; The same token model as tree-sitter-bullang's, so the two languages look like
; the family they are. The builtin list is BullScript's own — from
; lang/builtins.rs — and is much smaller than Bullang's.

"builtin" @keyword
"bag" @keyword

; Known builtins highlight as builtins; anything else is flagged, which is the
; same distinction the VS Code grammar and the Vim syntax make.
((builtin_call name: (identifier) @function.builtin)
 (#any-of? @function.builtin
  "add" "capture" "close" "i64_to_str" "in" "open" "out" "run"
  "str_to_i64" "to_lower" "to_upper" "trim"))

((builtin_call name: (identifier) @comment.error)
 (#not-any-of? @comment.error
  "add" "capture" "close" "i64_to_str" "in" "open" "out" "run"
  "str_to_i64" "to_lower" "to_upper" "trim"))

; A bag entry is a script you saved — a call, but not a builtin.
(bag_call name: (identifier) @function)

(input name: (identifier) @variable.parameter)
(binding name: (identifier) @variable)

(type) @type.builtin
(boolean) @boolean
(string) @string
(number) @number
(operator) @operator
"->" @operator
"::" @operator

[":" ";" ","] @punctuation.delimiter
["(" ")" "{" "}"] @punctuation.bracket

(comment) @comment
