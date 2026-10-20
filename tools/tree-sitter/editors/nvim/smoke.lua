-- Headless Neovim smoke for the parser and the queries. Run through
-- smoke.sh, which builds the parser and isolates Neovim from the
-- user's config and data. Exits 0 when every check holds.
--
--   nvim --headless --clean -l smoke.lua <parser.so> <runtime> <examples>

local parser, runtime, examples = arg[1], arg[2], arg[3]
vim.opt.runtimepath:prepend(runtime)
vim.treesitter.language.add('fipe', { path = parser })
vim.filetype.add({ extension = { fipe = 'fipe' } })

local failures = 0
local function check(ok, what)
  io.stdout:write((ok and 'ok   ' or 'FAIL ') .. what .. '\n')
  if not ok then
    failures = failures + 1
  end
end

local function open(name)
  vim.cmd('edit ' .. vim.fn.fnameescape(examples .. '/' .. name))
  local buf = vim.api.nvim_get_current_buf()
  vim.bo[buf].filetype = 'fipe'
  local tree = vim.treesitter.get_parser(buf, 'fipe'):parse()[1]
  vim.treesitter.start(buf, 'fipe')
  return buf, tree
end

-- The (row, column) of `text` in the first line holding `line`, a
-- snippet that names the line.
local function find(buf, line, text)
  for row, s in ipairs(vim.api.nvim_buf_get_lines(buf, 0, -1, true)) do
    local at = s:find(line, 1, true)
    if at then
      local col = s:find(text, at, true)
      assert(col, ('%q not in %q'):format(text, line))
      return row - 1, col - 1
    end
  end
  error(('no line holds %q'):format(line))
end

local function captured(buf, line, text, capture)
  local row, col = find(buf, line, text)
  for _, c in ipairs(vim.treesitter.get_captures_at_pos(buf, row, col)) do
    if c.capture == capture then
      return true
    end
  end
  return false
end

-- The deepest named node at (row, col) whose type is `kind`.
local function enclosing(tree, row, col, kind)
  local node = tree:root():named_descendant_for_range(row, col, row, col)
  while node and node:type() ~= kind do
    node = node:parent()
  end
  return node
end

-- Whether the indents query captures `node` as @indent.begin.
local function indent_begins(buf, tree, node)
  local query = vim.treesitter.query.get('fipe', 'indents')
  for id, n in query:iter_captures(tree:root(), buf, 0, -1) do
    if query.captures[id] == 'indent.begin' and n:id() == node:id() then
      return true
    end
  end
  return false
end

-- 04-text.fipe: no error node, `|:` an operator, a flag a parameter.
do
  local buf, tree = open('04-text.fipe')
  check(not tree:root():has_error(), '04-text.fipe has no error node')
  check(captured(buf, 'raw | utf8 |: len', '|:', 'operator'), '04-text.fipe: |: is @operator')
  check(captured(buf, 'sort -by', '-by', 'variable.parameter'),
    '04-text.fipe: -by is @variable.parameter')
  check(captured(buf, '| filter (w', 'filter', 'function.call'),
    '04-text.fipe: filter is @function.call')
  check(captured(buf, '# Text:', '#', 'comment'),
    '04-text.fipe: a comment is @comment')
end

-- 01-files.fipe: `-l` a parameter.
do
  local buf, tree = open('01-files.fipe')
  check(not tree:root():has_error(), '01-files.fipe has no error node')
  check(captured(buf, 'ls -l /var/log', '-l', 'variable.parameter'),
    '01-files.fipe: -l is @variable.parameter')
end

-- 03-types.fipe: `Point` a constructor, a fold over the arm block of
-- `area`, and the line after `area = self => self | {` indented.
do
  local buf, tree = open('03-types.fipe')
  check(not tree:root():has_error(), '03-types.fipe has no error node')
  check(captured(buf, 'p = Point 3 4', 'Point', 'constructor'),
    '03-types.fipe: Point in `p = Point 3 4` is @constructor')
  check(captured(buf, 'pub type Point', 'Point', 'type.definition'),
    '03-types.fipe: Point in its type item is @type.definition')
  check(captured(buf, '## A point with integer', '##', 'comment.documentation'),
    '03-types.fipe: a doc is @comment.documentation')

  vim.wo.foldmethod = 'expr'
  vim.wo.foldexpr = 'v:lua.vim.treesitter.foldexpr()'
  vim.cmd('normal! zx')
  local head = 'area = self => self | {'
  local row, col = find(buf, head, '{')
  local arms = enclosing(tree, row, col + 1, 'arms')
  local line = row + 1
  check(arms ~= nil and arms:start() == row,
    'the arm block of `area` is an arms node opening at its brace')
  check(tostring(vim.treesitter.foldexpr(line)):sub(1, 1) == '>',
    'a fold opens on the line of `area`')
  check(vim.fn.foldlevel(line + 1) > vim.fn.foldlevel(line - 1),
    'the arm block is folded: its first arm is a level deeper')

  row, col = find(buf, head, '=>')
  local arm = enclosing(tree, row, col, 'arm')
  check(arm ~= nil and arm:start() == row and arm:end_() > row
      and indent_begins(buf, tree, arm),
    'the arm `self => self | {` begins an indent over the next line')
end

-- 02-outcomes.fipe: the body after `load = path => {` is indented.
do
  local buf, tree = open('02-outcomes.fipe')
  check(not tree:root():has_error(), '02-outcomes.fipe has no error node')
  local row, col = find(buf, 'load = path => {', '=>')
  local arm = enclosing(tree, row, col, 'arm')
  check(arm ~= nil and indent_begins(buf, tree, arm)
      and arm:end_() > row,
    'the arm after `load = path =>` begins an indent over the lines below')
end

io.stdout:write(failures == 0 and 'smoke: all checks hold\n'
  or ('smoke: %d failed\n'):format(failures))
io.stdout:flush()
os.exit(failures == 0 and 0 or 1)
