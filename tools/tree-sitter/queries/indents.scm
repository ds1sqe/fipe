; Indents in nvim-treesitter capture names.

; Openers, the body after `=>`, and lines continuing a pipeline or an
; operator.
[
  (block)
  (arms)
  (list)
  (tuple)
  (group)
  (record)
  (type_body)
  (impl_item)
  (type_arguments)
  (type_parameters)
  (arm)
  (pipe)
  (binary)
  (function_type)
  (limits)
] @indent.begin

[
  "}"
  ")"
  "]"
] @indent.branch

[
  "}"
  ")"
  "]"
] @indent.end

[
  (comment)
  (doc)
] @indent.auto
