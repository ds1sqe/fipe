-- Fipe in Neovim: the `fipe` filetype, the language server, the
-- tree-sitter parser and queries, and an nvim-dap adapter. Load it
-- with `require('fipe').setup(opts)`; README.md lists the options.

local M = {}

-- tools/editor/nvim/lua/fipe/init.lua -> the repository root.
local source = debug.getinfo(1, 'S').source:sub(2)
local root = vim.fn.fnamemodify(source, ':p:h:h:h:h:h:h')
local grammar = root .. '/tools/tree-sitter'

local defaults = {
  -- fipe-lsp; false leaves the server out.
  lsp = root .. '/tools/lsp/target/release/fipe-lsp',
  -- fipe-dap; false leaves the adapter out.
  dap = root .. '/tools/dap/target/release/fipe-dap',
  -- The host declarations the server and the adapter check against.
  host = root .. '/tests/fixtures/host',
  -- The parser: false leaves tree-sitter out, nil builds it from
  -- tools/tree-sitter/src, a path loads that shared object.
  parser = nil,
  -- The trace fixture a launch replays: a function of the program's
  -- path, a path, or nil. The default is the conformance trace of the
  -- program's stem, when there is one.
  trace = function(program)
    local stem = vim.fn.fnamemodify(program, ':t:r')
    local path = root .. '/tests/fixtures/traces/' .. stem .. '.txt'
    return vim.uv.fs_stat(path) and path or nil
  end,
}

local function mtime(path)
  local stat = vim.uv.fs_stat(path)
  return stat and stat.mtime.sec or 0
end

-- Builds the parser under stdpath('cache') when it is missing or older
-- than its sources; nil and a message on failure.
local function build_parser()
  local src = grammar .. '/src'
  local sources = { src .. '/parser.c', src .. '/scanner.c' }
  local out = vim.fn.stdpath('cache') .. '/fipe/parser/fipe.so'
  if mtime(out) >= math.max(mtime(sources[1]), mtime(sources[2])) then
    return out
  end
  vim.fn.mkdir(vim.fn.fnamemodify(out, ':h'), 'p')
  local cc = vim.env.CC or 'cc'
  local cmd = { cc, '-shared', '-fPIC', '-O2', '-I', src, sources[1], sources[2], '-o', out }
  local result = vim.system(cmd, { text = true }):wait()
  if result.code ~= 0 then
    return nil, 'fipe: building the parser failed: ' .. result.stderr
  end
  return out
end

local function setup_treesitter(parser)
  local path, err = parser, nil
  if path == nil then
    path, err = build_parser()
  end
  if not path then
    vim.notify(err, vim.log.levels.WARN)
    return
  end
  vim.treesitter.language.add('fipe', { path = path })
  -- The queries are read from tools/tree-sitter/queries, not copied.
  for name, kind in vim.fs.dir(grammar .. '/queries') do
    if kind == 'file' and name:match('%.scm$') then
      local text = vim.fn.readfile(grammar .. '/queries/' .. name)
      vim.treesitter.query.set('fipe', name:sub(1, -5), table.concat(text, '\n'))
    end
  end
  vim.api.nvim_create_autocmd('FileType', {
    group = vim.api.nvim_create_augroup('fipe_treesitter', { clear = true }),
    pattern = 'fipe',
    callback = function(event)
      vim.treesitter.start(event.buf, 'fipe')
      for _, win in ipairs(vim.fn.win_findbuf(event.buf)) do
        vim.wo[win][0].foldmethod = 'expr'
        vim.wo[win][0].foldexpr = 'v:lua.vim.treesitter.foldexpr()'
      end
      -- Neovim has no tree-sitter indentexpr of its own; nvim-treesitter's
      -- reads the indents query.
      if pcall(require, 'nvim-treesitter') then
        vim.bo[event.buf].indentexpr = "v:lua.require'nvim-treesitter'.indentexpr()"
      end
    end,
  })
end

local function setup_lsp(opts)
  local config = {
    cmd = { opts.lsp, '--host', opts.host },
    filetypes = { 'fipe' },
    root_dir = function(buf, on_dir)
      on_dir(vim.fs.dirname(vim.api.nvim_buf_get_name(buf)))
    end,
    init_options = { host = opts.host },
  }
  local ok, cmp = pcall(require, 'cmp_nvim_lsp')
  if ok then
    config.capabilities = cmp.default_capabilities()
  end
  vim.lsp.config('fipe', config)
  vim.lsp.enable('fipe')
end

-- Registers the adapter and the launch configuration in nvim-dap; false
-- when nvim-dap is not installed.
local function register_dap(opts)
  local ok, dap = pcall(require, 'dap')
  if not ok then
    return false
  end
  dap.adapters.fipe = { type = 'executable', command = opts.dap }
  dap.configurations.fipe = {
    {
      type = 'fipe',
      request = 'launch',
      name = 'Run this file',
      program = '${file}',
      host = opts.host,
      -- nvim-dap calls a function value when the session starts.
      trace = function()
        local trace = opts.trace
        if type(trace) == 'function' then
          trace = trace(vim.api.nvim_buf_get_name(0))
        end
        return trace
      end,
    },
  }
  return true
end

-- nvim-dap is often lazy-loaded: register now when it is loaded, else
-- on the first fipe buffer.
local function setup_dap(opts)
  if package.loaded.dap then
    register_dap(opts)
    return
  end
  vim.api.nvim_create_autocmd('FileType', {
    group = vim.api.nvim_create_augroup('fipe_dap', { clear = true }),
    pattern = 'fipe',
    once = true,
    callback = function()
      register_dap(opts)
    end,
  })
end

--- Sets Fipe up; `opts` overrides the defaults above.
function M.setup(opts)
  opts = vim.tbl_extend('force', defaults, opts or {})
  M.options = opts
  vim.filetype.add({ extension = { fipe = 'fipe' } })
  if opts.parser ~= false then
    setup_treesitter(opts.parser)
  end
  if opts.lsp then
    setup_lsp(opts)
  end
  if opts.dap then
    setup_dap(opts)
  end
end

return M
