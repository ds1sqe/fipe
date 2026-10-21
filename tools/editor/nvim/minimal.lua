-- A minimal config for smoke.sh: the module and the installed plugins
-- it works with (nvim-dap, nvim-treesitter, cmp-nvim-lsp) on the
-- runtimepath, read from `FIPE_PLUGINS` (lazy.nvim's plugin root).
local here = vim.fn.fnamemodify(debug.getinfo(1, 'S').source:sub(2), ':p:h')
local plugins = vim.env.FIPE_PLUGINS or ''
for _, name in ipairs({ 'nvim-dap', 'nvim-treesitter', 'cmp-nvim-lsp' }) do
  if vim.uv.fs_stat(plugins .. '/' .. name) then
    vim.opt.runtimepath:append(plugins .. '/' .. name)
  end
end
vim.opt.runtimepath:prepend(here)
require('fipe').setup()
