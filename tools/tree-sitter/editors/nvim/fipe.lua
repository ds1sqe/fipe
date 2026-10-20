-- Fipe in Neovim: the filetype and the parser for nvim-treesitter.
-- Copy into your config, set `revision` to a commit of the language
-- repository, then run `:TSInstall fipe`. Queries: see README.md.

vim.filetype.add({ extension = { fipe = 'fipe' } })

local install_info = {
  url = 'https://github.com/ds1sqe/fipe',
  location = 'tools/tree-sitter',
  revision = 'COMMIT',
  -- nvim-treesitter `main` copies the queries from here.
  queries = 'tools/tree-sitter/queries',
}

local ok, parsers = pcall(require, 'nvim-treesitter.parsers')
if ok and parsers.get_parser_configs then
  -- nvim-treesitter `master`: the parser only; queries go under
  -- queries/fipe/ in your runtimepath.
  parsers.get_parser_configs().fipe = {
    install_info = {
      url = install_info.url,
      location = install_info.location,
      revision = install_info.revision,
      files = { 'src/parser.c', 'src/scanner.c' },
    },
    filetype = 'fipe',
  }
elseif ok then
  -- nvim-treesitter `main`.
  vim.api.nvim_create_autocmd('User', {
    pattern = 'TSUpdate',
    callback = function()
      require('nvim-treesitter.parsers').fipe = { install_info = install_info }
    end,
  })
end
