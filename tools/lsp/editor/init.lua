-- A Neovim config with fipe-lsp and nothing else: the module
-- tools/editor/nvim with the debug adapter left out. Use it as
-- `nvim -u tools/lsp/editor/init.lua`; for your own config, see
-- tools/editor/nvim/README.md. `FIPE_LSP` and `FIPE_HOST` override the
-- server and the host declarations.

local here = vim.fn.fnamemodify(debug.getinfo(1, "S").source:sub(2), ":p:h")
local root = vim.fn.fnamemodify(here, ":h:h:h")

vim.opt.runtimepath:prepend(root .. "/tools/editor/nvim")
require("fipe").setup({
  lsp = vim.env.FIPE_LSP,
  host = vim.env.FIPE_HOST,
  dap = false,
})
