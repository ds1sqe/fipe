; Scopes, definitions and references in nvim-treesitter capture names.

; A file, a group, a block, an arm and a `with` body are scopes
; (02-syntax.md §Bindings, §Arms).
[
  (file)
  (group)
  (block)
  (arm)
  (with)
] @local.scope

; ---- Definitions -----------------------------------------------------

(binding
  pattern: (name) @local.definition.function
  value: (arms))

(binding
  pattern: (name) @local.definition.var)

(binding
  pattern: (_
    (name) @local.definition.var))

(arm
  pattern: (name) @local.definition.parameter)

(arm
  pattern: (_
    (name) @local.definition.parameter))

(default_pattern
  name: (name) @local.definition.parameter)

(pattern_field
  name: (name) @local.definition.var
  !pattern)

(with
  key: (name) @local.definition.var)

(type_item
  name: (_) @local.definition.type)

(type_parameter
  name: (_) @local.definition.type)

(import
  name: (_) @local.definition.import)

; ---- References ------------------------------------------------------

(name) @local.reference
