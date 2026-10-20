; Injections in nvim-treesitter capture names.

; Comments and docs carry the comment language (TODO, NOTE and links).
([
  (comment)
  (doc)
  (inner_doc)
] @injection.content
  (#set! injection.language "comment"))
