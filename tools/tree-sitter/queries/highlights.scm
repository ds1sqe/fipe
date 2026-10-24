; Highlights in nvim-treesitter capture names. Later patterns win over
; earlier ones for the same node, so the general captures come first.

; ---- Names -----------------------------------------------------------

(name) @variable

; The case rule (02-syntax.md): an uppercase name in an expression or a
; pattern is a constructor.
(upper_name) @constructor

((upper_name) @boolean
  (#any-of? @boolean "True" "False"))

; ---- Keywords --------------------------------------------------------

"use" @keyword.import

[
  "type"
  "impl"
] @keyword.type

"pub" @keyword.modifier

(type_item
  resource: (resource_modifier) @keyword.modifier)

[
  "if"
  "else"
] @keyword.conditional

[
  "with"
  "where"
] @keyword

; ---- Operators and punctuation ---------------------------------------

; The pipes are operators (02-syntax.md §Pipelines).
[
  "|"
  "|:"
  "|!"
  "||"
  "&&"
] @operator

[
  "+"
  "-"
  "*"
  "/"
  "%"
  "++"
  "=="
  "!="
  "<"
  "<="
  ">"
  ">="
  "!"
  "&"
  "="
  "=>"
  "->"
] @operator

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

(type_arguments
  [
    "<"
    ">"
  ] @punctuation.bracket)

(type_parameters
  [
    "<"
    ">"
  ] @punctuation.bracket)

[
  ","
  ";"
  ":"
] @punctuation.delimiter

; `..`, the rest of a record pattern.
(rest_pattern) @punctuation.special

; ---- Literals --------------------------------------------------------

(integer) @number

(float) @number.float

[
  (string)
  (raw_string)
] @string

(path) @string.special.path

; A word that is not a name is text (01-lexical.md §Word
; classification).
(word) @string.special

(comment) @comment @spell

[
  (doc)
  (inner_doc)
] @comment.documentation @spell

; ---- Modules ---------------------------------------------------------

(import
  name: (_) @module)

(version) @number

; ---- Types -----------------------------------------------------------

(named_type
  name: (_) @type)

(type_parameter
  name: (_) @type)

(type_item
  name: (_) @type.definition)

(unit_type) @type

; ---- Members and fields ----------------------------------------------

(field_declaration
  name: (_) @variable.member)

(member_limit
  name: (_) @variable.member)

(field_value
  name: (name) @variable.member)

(pattern_field
  name: (name) @variable.member)

(field_name) @variable.member

(selector) @variable.member

(type_body
  (constructor
    name: (_) @constructor))

; ---- Functions -------------------------------------------------------

; A binding whose value is an arm list is a function (02-syntax.md
; §Arms).
(binding
  pattern: (name) @function
  value: (arms))

(impl_member
  name: (_) @function
  value: (arms))

(declaration
  name: (_) @function
  type: (function_type))

(declaration
  name: (_) @function
  type: (qualified_type
    type: (function_type)))

(impl_member
  name: (_) @function
  type: [
    (function_type)
    (qualified_type)
  ])

(application
  callee: (name) @function.call)

(call
  callee: (name) @function.call)

; ---- Parameters ------------------------------------------------------

; A flag is a named argument (02-syntax.md §Applications).
(flag) @variable.parameter

(arm
  pattern: (name) @variable.parameter)

(default_pattern
  name: (name) @variable.parameter)

(labeled_type
  name: (name) @variable.parameter)
