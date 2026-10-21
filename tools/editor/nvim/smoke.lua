-- The module's headless smoke, run by smoke.sh after a config that
-- calls require('fipe').setup():
--   nvim --headless [-u minimal.lua] -l smoke.lua <root> <scratch>
-- Each step prints PASS or FAIL; the exit status is 0 only if all pass.

local root, scratch = arg[1], arg[2]
-- Generous: the server's first start can be slow on a loaded machine.
local timeout = tonumber(vim.env.FIPE_SMOKE_TIMEOUT) or 60000
local failures = 0

local function step(ok, what)
  io.stdout:write((ok and 'PASS ' or 'FAIL ') .. what .. '\n')
  io.stdout:flush()
  if not ok then
    failures = failures + 1
  end
  return ok
end

local function finish()
  io.stdout:write(failures == 0 and 'smoke: all steps pass\n' or ('smoke: %d failed\n'):format(failures))
  io.stdout:flush()
  os.exit(failures == 0 and 0 or 1)
end

local function attached(buf)
  return vim.wait(timeout, function()
    local client = vim.lsp.get_clients({ bufnr = buf, name = 'fipe' })[1]
    return client ~= nil and client.initialized
  end, 50)
end

-- The language server on the example.
local example = root .. '/Documentation/examples/05-modules.fipe'
vim.cmd.edit(vim.fn.fnameescape(example))
local buf = vim.api.nvim_get_current_buf()
step(vim.bo[buf].filetype == 'fipe', '05-modules.fipe opens as filetype fipe')
step(attached(buf), 'the fipe server attaches to 05-modules.fipe')

-- The grammar's captures: `use` on line 4 is @keyword.import.
local function captured(b, row, col, name)
  for _, c in ipairs(vim.treesitter.get_captures_at_pos(b, row, col)) do
    if c.capture == name then
      return true
    end
  end
  return false
end
step(vim.treesitter.highlighter.active[buf] ~= nil, 'tree-sitter highlights the buffer')
step(captured(buf, 3, 0, 'keyword.import'), '`use` at 4:1 is @keyword.import')
step(captured(buf, 10, 0, 'comment.documentation'), '`##` at 11:1 is @comment.documentation')
step(vim.wo.foldexpr == 'v:lua.vim.treesitter.foldexpr()', 'foldexpr is the tree-sitter fold')
step(vim.treesitter.query.get('fipe', 'indents') ~= nil, 'the indents query is loaded')

-- A diagnostic on a scratch file with an error.
local bad = scratch .. '/bad.fipe'
vim.fn.writefile({ 'bad = find ./x | cat' }, bad)
vim.cmd.edit(vim.fn.fnameescape(bad))
local badbuf = vim.api.nvim_get_current_buf()
step(attached(badbuf), 'the fipe server attaches to the scratch file')
local seen = vim.wait(timeout, function()
  return #vim.diagnostic.get(badbuf, { severity = vim.diagnostic.severity.ERROR }) > 0
end, 50)
local first = vim.diagnostic.get(badbuf)[1]
step(seen and first.lnum == 0, 'an error diagnostic on line 1 of the scratch file')
io.stdout:write('     ' .. (first and first.message or '') .. '\n')

-- nvim-dap: a breakpoint on `e.name` (line 25), then the end.
vim.cmd.edit(vim.fn.fnameescape(example))
buf = vim.api.nvim_get_current_buf()
local ok, dap = pcall(require, 'dap')
if not step(ok and dap.adapters.fipe ~= nil, 'nvim-dap has the fipe adapter') then
  finish()
end
step(dap.configurations.fipe and #dap.configurations.fipe > 0, 'nvim-dap has a fipe launch configuration')

local events = {}
dap.listeners.after.event_stopped.fipe_smoke = function(_, body)
  table.insert(events, { 'stopped', body })
end
dap.listeners.after.event_terminated.fipe_smoke = function()
  table.insert(events, { 'terminated' })
end
local function wait_for(name)
  return vim.wait(timeout, function()
    for _, e in ipairs(events) do
      if e[1] == name then
        return true
      end
    end
    return false
  end, 50)
end

-- A config may open dap-ui on launch; headless Neovim has no screen
-- for its layout (the README's checklist covers dap-ui by hand).
for _, when in ipairs({ 'before', 'after' }) do
  for _, listeners in pairs(dap.listeners[when]) do
    listeners.dapui_config = nil
  end
end

require('dap.breakpoints').set({}, buf, 25)
dap.run(dap.configurations.fipe[1])
local stopped = wait_for('stopped')
-- nvim-dap asks for the stack after the event; wait for its frame.
vim.wait(timeout, function()
  return dap.session() ~= nil and dap.session().current_frame ~= nil
end, 50)
local frame = dap.session() and dap.session().current_frame
step(stopped and events[1][2].reason == 'breakpoint', 'the session stops at the breakpoint')
step(frame ~= nil and frame.line == 25, 'the top frame is on line 25 (' .. (frame and frame.line or '?') .. ')')
dap.continue()
step(wait_for('terminated'), 'continuing runs the program to its end (terminated)')
finish()
