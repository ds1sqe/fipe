-- The editor smoke: run by smoke.sh as
--   nvim --headless -u tools/lsp/editor/init.lua -l tools/lsp/editor/smoke.lua
-- It opens 05-modules.fipe, sees a located check error after an edit,
-- jumps to a used module's definition and exits 0; any failed check
-- exits 1. Each step prints one line.

local here = vim.fn.fnamemodify(debug.getinfo(1, "S").source:sub(2), ":p:h")
local root = vim.fn.fnamemodify(here, ":h:h:h")
local example = root .. "/Documentation/examples/05-modules.fipe"
local timeout = 20000

local function check(ok, what)
  if ok then
    print("ok   " .. what)
  else
    print("FAIL " .. what)
    vim.cmd("cquit 1")
  end
end

vim.cmd.edit(example)
local buf = vim.api.nvim_get_current_buf()
check(vim.bo[buf].filetype == "fipe", "05-modules.fipe opens as fipe")

local attached = vim.wait(timeout, function()
  return #vim.lsp.get_clients({ bufnr = buf, name = "fipe-lsp" }) > 0
    and vim.lsp.get_clients({ bufnr = buf })[1].initialized
end, 50)
check(attached, "fipe-lsp attaches to the buffer")

local function errors()
  return vim.diagnostic.get(buf, { severity = vim.diagnostic.severity.ERROR })
end

-- The example checks, so the first publish holds no error.
vim.wait(2000, function() return false end, 50)
check(#errors() == 0, "the example has no error diagnostic")

-- An edit: a line whose stage needs an Entry and gets a Maybe<Entry>.
local last = vim.api.nvim_buf_line_count(buf)
vim.api.nvim_buf_set_lines(buf, last, last, false, { "bad = find ./x | cat" })
local seen = vim.wait(timeout, function() return #errors() > 0 end, 50)
check(seen, "the edit publishes a check error")
local first = errors()[1]
check(
  first ~= nil and first.lnum == last,
  "the error is located on the edited line (line " .. (last + 1) .. ")"
)
print("     " .. (first and first.message or ""))

-- Undo the edit: the error clears in the next publish.
vim.api.nvim_buf_set_lines(buf, last, last + 1, false, {})
local cleared = vim.wait(timeout, function() return #errors() == 0 end, 50)
check(cleared, "removing the line clears the error")

-- Definition on `Point` in `p: Point = …`, a name `use geometry.Point`
-- brought in from the file module geometry.fipe.
local row
for i, line in ipairs(vim.api.nvim_buf_get_lines(buf, 0, -1, false)) do
  if line:find("p: Point", 1, true) then row = i break end
end
check(row ~= nil, "a use of Point is found")
local col = vim.api.nvim_buf_get_lines(buf, row - 1, row, false)[1]:find("Point", 1, true) - 1
vim.api.nvim_win_set_cursor(0, { row, col })
local client = vim.lsp.get_clients({ bufnr = buf, name = "fipe-lsp" })[1]
local params = vim.lsp.util.make_position_params(0, client.offset_encoding)
local answers = vim.lsp.buf_request_sync(buf, "textDocument/definition", params, timeout)
local location
for _, answer in pairs(answers or {}) do
  local result = answer.result
  if result and result.uri then location = result
  elseif result and result[1] then location = result[1] end
end
check(location ~= nil, "definition answers a location")
check(
  location ~= nil and location.uri:match("geometry%.fipe$") ~= nil,
  "the definition is in geometry.fipe"
)
vim.lsp.util.show_document(location, client.offset_encoding, { focus = true })
local name = vim.api.nvim_buf_get_name(0)
local cursor = vim.api.nvim_win_get_cursor(0)
local text = vim.api.nvim_buf_get_lines(0, cursor[1] - 1, cursor[1], false)[1]
check(name:match("geometry%.fipe$") ~= nil, "the jump opens geometry.fipe")
check(text:find("Point", 1, true) ~= nil, "the cursor is on Point's line: " .. vim.trim(text))

print("smoke: all checks hold")
vim.cmd("qall!")
