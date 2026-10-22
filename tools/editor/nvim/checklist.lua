-- Probes for checklist.sh, run inside the Neovim under test through
-- `nvim --server … --remote-expr`: each returns a short string the
-- script compares. They read state; the keys are sent through tmux.

local C = {}

-- 1 when the fipe server is attached to the current buffer.
function C.attached()
  local client = vim.lsp.get_clients({ bufnr = 0, name = 'fipe' })[1]
  return (client and client.initialized) and 1 or 0
end

function C.mode()
  return vim.api.nvim_get_mode().mode
end

-- "<file>:<line>:<word under the cursor>".
function C.where()
  local line = vim.api.nvim_win_get_cursor(0)[1]
  local name = vim.fn.fnamemodify(vim.api.nvim_buf_get_name(0), ':t')
  return ('%s:%d:%s'):format(name, line, vim.fn.expand('<cword>'))
end

-- The text of every floating window, one line each.
function C.floats()
  local out = {}
  for _, win in ipairs(vim.api.nvim_list_wins()) do
    if vim.api.nvim_win_get_config(win).relative ~= '' then
      local buf = vim.api.nvim_win_get_buf(win)
      vim.list_extend(out, vim.api.nvim_buf_get_lines(buf, 0, -1, false))
    end
  end
  return table.concat(out, '\n')
end

-- Lines a..b of the current buffer, joined with newlines.
function C.lines(a, b)
  return table.concat(vim.api.nvim_buf_get_lines(0, a - 1, b, false), '\n')
end

-- The labels nvim-cmp's open menu shows from the fipe server, sorted.
function C.cmp()
  local ok, cmp = pcall(require, 'cmp')
  if not ok or not cmp.visible() then
    return ''
  end
  local labels = {}
  for _, entry in ipairs(cmp.get_entries()) do
    if entry.source.name == 'nvim_lsp' then
      table.insert(labels, entry.completion_item.label)
    end
  end
  table.sort(labels)
  return table.concat(labels, ',')
end

function C.foldclosed(line)
  return vim.fn.foldclosed(line)
end

-- The first error diagnostic's message on a line, or "".
function C.error(line)
  local d = vim.diagnostic.get(0, { lnum = line - 1, severity = vim.diagnostic.severity.ERROR })[1]
  return d and d.message or ''
end

-- "stopped:<line>", "running" or "none".
function C.dap()
  local ok, dap = pcall(require, 'dap')
  local session = ok and dap.session()
  if not session then
    return 'none'
  end
  local frame = session.current_frame
  return (session.stopped_thread_id and frame) and ('stopped:' .. frame.line) or 'running'
end

-- How many nvim-dap breakpoints are set.
function C.breakpoints()
  local ok, breakpoints = pcall(require, 'dap.breakpoints')
  if not ok then
    return 0
  end
  local n = 0
  for _, list in pairs(breakpoints.get()) do
    n = n + #list
  end
  return n
end

-- How many times nvim-dap has stopped since the first call; the
-- count lives in nvim-dap's table, as each probe call reloads this file.
function C.stops()
  local dap = require('dap')
  dap.listeners.after.event_stopped.fipe_checklist = function()
    dap.fipe_checklist_stops = (dap.fipe_checklist_stops or 0) + 1
  end
  return dap.fipe_checklist_stops or 0
end

-- The dap-ui element filetypes shown in a window, sorted and unique.
function C.dapui()
  local seen = {}
  for _, win in ipairs(vim.api.nvim_list_wins()) do
    local ft = vim.bo[vim.api.nvim_win_get_buf(win)].filetype
    if ft:match('^dapui_') then
      seen[ft] = true
    end
  end
  local out = vim.tbl_keys(seen)
  table.sort(out)
  return table.concat(out, ',')
end

-- The scope names dap-ui's scopes element lists, in order.
function C.scopes()
  for _, win in ipairs(vim.api.nvim_list_wins()) do
    local buf = vim.api.nvim_win_get_buf(win)
    if vim.bo[buf].filetype == 'dapui_scopes' then
      local names = {}
      for _, line in ipairs(vim.api.nvim_buf_get_lines(buf, 0, -1, false)) do
        table.insert(names, line:match('^%W*(%u%a+):%s*$'))
      end
      return table.concat(names, ',')
    end
  end
  return ''
end

-- The 24-bit foreground the highlighter gives a buffer position: the
-- last tree-sitter capture there with a foreground, as "#rrggbb".
local function want_fg(line, col)
  local fg, group
  for _, c in ipairs(vim.inspect_pos(0, line - 1, col - 1).treesitter) do
    local hl = vim.api.nvim_get_hl(0, { name = c.hl_group, link = false })
    if hl.fg then
      fg, group = hl.fg, c.hl_group
    end
  end
  return fg and ('#%06x'):format(fg), group
end

-- The foreground of screen cell (row, col), 1-based, in a
-- `tmux capture-pane -e` dump: the last SGR foreground before it.
local function cell_fg(dump, row, col)
  local text = vim.fn.readfile(dump)[row] or ''
  local fg, cells, i = nil, 0, 1
  while i <= #text do
    local params, stop = text:match('^\27%[([%d;]*)m()', i)
    if params then
      local p = vim.split(params, ';')
      local k = 1
      while k <= #p do
        local n = tonumber(p[k]) or 0
        if n == 38 and p[k + 1] == '2' then
          fg = ('#%02x%02x%02x'):format(p[k + 2], p[k + 3], p[k + 4])
          k = k + 4
        elseif n == 48 and p[k + 1] == '2' then
          k = k + 4
        elseif n == 0 or n == 39 then
          fg = nil
        end
        k = k + 1
      end
      i = stop
    else
      local char = text:match('^[%z\1-\127\194-\244][\128-\191]*', i)
      cells = cells + vim.fn.strdisplaywidth(char)
      if cells >= col then
        return fg
      end
      i = i + #char
    end
  end
end

-- "ok <group> <fg>" when the screen draws buffer position (line, col)
-- in its capture's colour, else what differs.
function C.colour(dump, line, col)
  local want, group = want_fg(line, col)
  local pos = vim.fn.screenpos(0, line, col)
  local got = cell_fg(dump, pos.row, pos.col)
  if want and got == want then
    return ('ok %s %s'):format(group, want)
  end
  return ('want %s (%s), screen %s'):format(tostring(want), tostring(group), tostring(got))
end

return C
