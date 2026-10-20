-- A Neovim client for fipe-lsp with the built-in `vim.lsp`, no plugin.
-- Use it as `nvim -u tools/lsp/editor/init.lua`, or copy the body into
-- your own configuration with your paths. The server is the release
-- build of this package; host declarations come from `FIPE_HOST`, or
-- the conformance host of the repository.

local here = vim.fn.fnamemodify(debug.getinfo(1, "S").source:sub(2), ":p:h")
local package = vim.fn.fnamemodify(here, ":h")
local root = vim.fn.fnamemodify(package, ":h:h")
local server = vim.env.FIPE_LSP or (package .. "/target/release/fipe-lsp")
local host = vim.env.FIPE_HOST or (root .. "/tests/fixtures/host")

vim.filetype.add({ extension = { fipe = "fipe" } })

vim.api.nvim_create_autocmd("FileType", {
  pattern = "fipe",
  callback = function(event)
    vim.lsp.start({
      name = "fipe-lsp",
      cmd = { server, "--host", host },
      root_dir = vim.fs.dirname(vim.api.nvim_buf_get_name(event.buf)),
      init_options = { host = host },
    }, { bufnr = event.buf })
  end,
})
