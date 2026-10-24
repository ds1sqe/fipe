/**
 * @file Fipe grammar for tree-sitter
 * @author Kim Dae Up
 * @license MIT OR Apache-2.0
 *
 * Mirrors Documentation/spec/01-lexical.ebnf and 02-syntax.ebnf in the
 * language repository; the chapters there are normative. Each regex
 * below names its lexical rule, and each conflict or precedence its
 * syntax rule. The scanner (src/scanner.c) owns what a regular lexer
 * cannot see: newlines, line joins, comments, quotes and the pipes.
 */

/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

// 01-lexical.md §Boundaries: spacing, LF, controls, U+FEFF, U+200B and
// ( ) { } [ ] , ; : = < > ! | & " ' # end a word. The backslash is a
// word character except where it begins a line join, which only the
// scanner can see; inside a run here it must be followed by a word
// character, and trailing backslashes end the run.
const BOUNDARY =
  String.raw`\x00-\x20\x7F-\xA0\u1680\u2000-\u200B\u2028\u2029\u202F` +
  String.raw`\u205F\u3000\uFEFF()\[\]{},;:=<>!|&"'#`;
const SCALAR = String.raw`[^${BOUNDARY}\\]`;
const BODY = String.raw`(?:${SCALAR}|\\+${SCALAR})`;
const TRAIL = String.raw`\\*`;

// 01-lexical.md §Word classification: a name segment.
const IDENT = String.raw`[A-Za-z_][A-Za-z0-9_]*`;
const QUALIFIED = String.raw`${IDENT}(?:\.${IDENT})*`;

// 01-lexical.md §Numbers. Digits with single underscores between them.
const DEC = String.raw`[0-9](?:_?[0-9])*`;
const HEX = String.raw`[0-9a-fA-F](?:_?[0-9a-fA-F])*`;
const OCT = String.raw`[0-7](?:_?[0-7])*`;
const BIN = String.raw`[01](?:_?[01])*`;
const EXP = String.raw`[eE][+-]?${DEC}`;
// A suffix is a letter, then letters, digits and underscores (rule 4).
// Read in order, the longest number comes first, so a suffix cannot
// start with what would extend the number: a digit of the radix, or an
// exponent after a decimal; after a lone `0`, not `x`, `o` or `b`
// either (rule 1 makes those words when no digit follows).
const SUFFIX = String.raw`[A-Za-z][A-Za-z0-9_]*`;
const SUFFIX_HEX = String.raw`[g-zG-Z][A-Za-z0-9_]*`;
const NOT_EXPONENT = String.raw`[eE](?:[A-Za-z_][A-Za-z0-9_]*)?`;
const SUFFIX_DEC = String.raw`(?:[A-DF-Za-df-z][A-Za-z0-9_]*|${NOT_EXPONENT})`;
const SUFFIX_ZERO =
  String.raw`(?:[A-DF-Za-ac-df-np-wyz][A-Za-z0-9_]*|${NOT_EXPONENT})`;

const INTEGER = String.raw`-?(?:` +
  [
    String.raw`0x${HEX}(?:${SUFFIX_HEX})?`,
    String.raw`0o${OCT}(?:${SUFFIX})?`,
    String.raw`0b${BIN}(?:${SUFFIX})?`,
    String.raw`0(?:_?[0-9])+(?:${SUFFIX_DEC})?`,
    String.raw`[1-9](?:_?[0-9])*(?:${SUFFIX_DEC})?`,
    // A lone `0`, a program's prefix (rule 3) or a suffix (rule 4).
    String.raw`0(?:${SUFFIX_ZERO})?`,
  ].join('|') +
  ')';
const FLOAT = String.raw`-?(?:` +
  [
    String.raw`${DEC}\.${DEC}(?:${SUFFIX_DEC})?`,
    String.raw`${DEC}\.${DEC}${EXP}(?:${SUFFIX})?`,
    String.raw`${DEC}${EXP}(?:${SUFFIX})?`,
  ].join('|') +
  ')';

const KEYWORDS = ['if', 'else', 'with', 'type', 'impl', 'use', 'pub'];
// The five pipes: the scanner owns them, since the first LF after one
// is suppressed (01-lexical.md §Comments and logical lines).
const PIPES = ['|', '|:', '|!', '||', '&&'];

// 01-lexical.md §Word classification: `+ - * / % ++` as whole words are
// symbols, so a word is any other run. Since the lexer only considers
// tokens the parser can take, the word regex itself must not spell
// them: one unit not a symbol, two units not `++`, three or more, or a
// symbol before trailing backslashes; then the trailing backslashes.
const SYMBOL_CHAR = String.raw`+\-*/%`;
const UNIT_NOT_SYMBOL =
  String.raw`(?:[^${BOUNDARY}\\${SYMBOL_CHAR}]|\\+${SCALAR})`;
const UNIT_NOT_PLUS = String.raw`(?:[^${BOUNDARY}\\+]|\\+${SCALAR})`;
const WORD =
  String.raw`(?:${UNIT_NOT_SYMBOL}|${UNIT_NOT_PLUS}${BODY}|\+` +
  String.raw`${UNIT_NOT_PLUS}|${BODY}${BODY}${BODY}+)${TRAIL}` +
  String.raw`|(?:\+\+|[${SYMBOL_CHAR}])\\+|\\+`;

// 01-lexical.md §Word classification: a selector, and a name whose last
// segment's case the case rule reads (02-syntax.md, the case rule).
const SELECTOR = String.raw`\.${QUALIFIED}`;
const LOWER = String.raw`(?:${IDENT}\.)*[a-z_][A-Za-z0-9_]*`;
const UPPER = String.raw`(?:${IDENT}\.)*[A-Z][A-Za-z0-9_]*`;

// 02-syntax.md §Precedence, loosest first.
const PREC = {
  pipe: 1,
  equality: 2,
  comparison: 3,
  sum: 4,
  product: 5,
  not: 6,
  postfix: 7,
  record: 8,
  // `[` then `:` or `!` is an outcome bracket, never a list.
  outcome: 9,
};

const ARITHMETIC = [
  [['*', '/', '%'], PREC.product],
  [['+', '-', '++'], PREC.sum],
];

/**
 * The value levels of 02-syntax.md §Precedence, as hidden rules aliased
 * to `binary` and `not`: arithmetic is left-associative, a comparison
 * takes arithmetic operands and an equality comparison operands, so
 * neither chains. `prefix` names a family; `operand` its operand rule.
 *
 * @param {string} prefix
 * @param {string} operand
 */
function values(prefix, operand) {
  const rule = name => `_${prefix}${name}`;
  return {
    [rule('value')]: $ => choice(
      $[rule('comparison_value')],
      alias($[rule('equality')], $.binary),
    ),
    [rule('equality')]: $ => prec(PREC.equality, seq(
      field('left', $[rule('comparison_value')]),
      field('operator', choice('==', '!=')),
      field('right', $[rule('comparison_value')]),
    )),
    [rule('comparison_value')]: $ => choice(
      $[rule('arithmetic_value')],
      alias($[rule('comparison')], $.binary),
    ),
    [rule('comparison')]: $ => prec(PREC.comparison, seq(
      field('left', $[rule('arithmetic_value')]),
      field('operator', choice('<', '<=', '>', '>=')),
      field('right', $[rule('arithmetic_value')]),
    )),
    [rule('arithmetic_value')]: $ => choice(
      alias($[rule('arithmetic')], $.binary),
      alias($[rule('not')], $.not),
      $[operand],
    ),
    [rule('arithmetic')]: $ => choice(
      ...ARITHMETIC.map(([operators, precedence]) =>
        prec.left(precedence, seq(
          field('left', $[rule('arithmetic_value')]),
          field('operator', choice(...operators)),
          field('right', $[rule('arithmetic_value')]),
        )),
      ),
    ),
    // unary = { "!" }, operand.
    [rule('not')]: $ => prec(PREC.not, seq(
      field('operator', '!'),
      field('value', choice(alias($[rule('not')], $.not), $[operand])),
    )),
  };
}

/**
 * A comma list (02-syntax.md §Lists and blocks): `gap` (newlines, and
 * docs where an item may carry one) only after the opener, after a
 * comma and before the closer, and a trailing comma.
 *
 * @param {RuleOrLiteral} open
 * @param {RuleOrLiteral} item
 * @param {RuleOrLiteral} close
 * @param {RuleOrLiteral} gap
 * @param {RuleOrLiteral} comma
 */
function list(open, item, close, gap, comma) {
  return seq(
    open,
    repeat(gap),
    optional(seq(item, repeat(seq(comma, item)), choice(comma, repeat(gap)))),
    close,
  );
}

export default grammar({
  name: 'fipe',

  externals: $ => [
    $._newline,
    $._continuation,
    $._join,
    $.comment,
    $.doc,
    $.inner_doc,
    $.string,
    $.raw_string,
    ...PIPES,
    // A run of word characters, classified by the scanner as the crate's
    // lexer classifies it, whatever the parser expects.
    $.integer,
    $.float,
    $.flag,
    $.path,
    $.selector,
    $.name,
    $.word,
    '+', '-', '*', '/', '%', '++',
    ...KEYWORDS,
    $.upper_name,
    'where',
    // A selector touching the token before it.
    $.field_name,
    $.resource_modifier,
  ],

  extras: $ => [
    // 01-lexical.md §Source and positions: spacing, and the byte-order
    // mark the lexer skips at offset zero.
    /[\t\r \u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000\uFEFF]/,
    $.comment,
    $._continuation,
    $._join,
  ],

  word: $ => $.name,

  // The keywords are never names (01-lexical.md §Word classification);
  // `where` is a name outside a `type` item, so it is not reserved.
  reserved: {
    global: _ => KEYWORDS,
  },

  // GLR forks, each settled where its syntax rule says.
  conflicts: $ => [
    // The binding and arm rules (02-syntax.md §Bindings, §Arms): a line
    // is a binding when a top-level `=` comes before its first `=>`,
    // pipe, comma or newline, and an arm list when a head comes before a
    // `=>`, so pattern and operand fork until one of them.
    [$._primary, $._pattern_atom],
    [$._primary, $.constructor_pattern],
    [$._primary, $.constructor_pattern, $._pattern_atom],
    [$.list, $.list_pattern],
    [$.outcome, $.outcome_pattern],
    [$.field_value, $.pattern_field],
    [$.default_pattern, $._pattern_atom],
    // A file-level `name: type` with no `=` is a declaration
    // (§Definitions, declaration forms).
    [$.declaration, $._pattern_atom],
    // The case rule: an uppercase name in a pattern takes one payload,
    // so `Just e rest` is two patterns; dynamic precedence on
    // `constructor_pattern` takes the payload.
    [$.constructor_pattern, $._pattern_atom],
    // A comma carries one newline over in an arm list (§Arms); dynamic
    // precedence keeps the list going where the next line could also be
    // a line of its own.
    [$._arm_list],
    // A braced or parenthesized body whose first line has a top-level
    // `=>` is an arm list (§Arms), preferred by dynamic precedence to a
    // block or group holding an arm stage.
    [$._braced_arms, $.arms],
    [$.block, $._braced_arms],
    [$._parenthesized_arms, $.arms],
    // Where a comma separates items an element takes no arm list
    // (§Precedence), so a group's line and a tuple's element fork until
    // a `,` or `)`.
    [$._pipeline_element, $._stage],
    // `/` after an operand is division or a root-path argument
    // (§Precedence); division wins by dynamic precedence.
    [$.application, $._arithmetic_value],
    [$._head_application, $._head_arithmetic_value],
  ],

  rules: {
    // file = newlines, [ inner_doc ], { entry, NEWLINE, newlines },
    // [ entry ], EOF. A doc is a line of its own before its owner.
    file: $ => seq(
      repeat(choice(
        $._newline, $.doc, $.inner_doc, seq($._entry, $._newline),
      )),
      optional($._entry),
    ),

    // entry = [ doc ], [ "pub" ], ( item | declaration | line ). `pub`
    // stands outside the node it exports, as in the crate's spans.
    _entry: $ => choice(
      $.import,
      $.type_item,
      $.impl_item,
      $.declaration,
      $._line,
      seq('pub', choice($.binding, $.type_item, $.declaration)),
    ),

    _comma: $ => seq(',', repeat($._newline)),
    _comma_doc: $ => seq(',', repeat(choice($._newline, $.doc))),

    // ---- Items -------------------------------------------------------

    // import = "use", NAME, { NAME }, [ INTEGER ].
    import: $ => seq(
      'use',
      repeat1(field('name', choice($.name, $.upper_name))),
      optional(field('version', alias($.integer, $.version))),
    ),

    // declaration = NAME, ":", annotation.
    declaration: $ => seq(
      field('name', choice($.name, $.upper_name)),
      ':',
      field('type', $._annotation),
    ),

    // type_item = "type", [ contextual resource modifier ], NAME, ... .
    type_item: $ => seq(
      'type',
      optional(field('resource', $.resource_modifier)),
      field('name', choice($.upper_name, $.name)),
      optional(field('parameters', $.type_parameters)),
      optional(choice(
        seq(
          optional(seq(':', field('base', $._type))),
          field('members', $.type_body),
          optional($.limits),
        ),
        seq(':', field('base', $._type), optional($.limits)),
        $.limits,
      )),
    ),

    type_body: $ => list(
      '{', $._member, '}', choice($._newline, $.doc), $._comma_doc,
    ),

    // member = [ doc ], ( field | NAME, [ "(", fields, ")" ] ).
    _member: $ => choice($.field_declaration, $.constructor),

    constructor: $ => seq(
      field('name', choice($.upper_name, $.name)),
      optional(list(
        token.immediate('('), $.field_declaration, ')', $._newline,
        $._comma,
      )),
    ),

    // field = [ "pub" ], NAME, ":", type, [ "=", operand ].
    field_declaration: $ => seq(
      optional('pub'),
      field('name', choice($.name, $.upper_name)),
      ':',
      field('type', $._type),
      optional(seq('=', field('default', $._operand))),
    ),

    // limits = "where", newlines, limit, { ",", newlines, limit }.
    limits: $ => seq(
      'where',
      repeat($._newline),
      $._limit,
      repeat(seq($._comma, $._limit)),
    ),

    _limit: $ => choice($.member_limit, $._pipeline_element),

    member_limit: $ => seq(
      field('name', $.name),
      ':',
      field('type', $._type),
    ),

    type_parameters: $ => list(
      '<', $.type_parameter, '>', $._newline, $._comma,
    ),

    // type_parameter = NAME, [ ":", type_both ], [ "=", type ].
    type_parameter: $ => seq(
      field('name', choice($.upper_name, $.name)),
      optional(seq(':', field('bound', $._type_both))),
      optional(seq('=', field('default', $._type))),
    ),

    // impl_item = "impl", type, [ ":", type_atom ], "{", impl_members,
    // "}".
    impl_item: $ => seq(
      'impl',
      field('target', $._type),
      optional(seq(':', field('claim', $._type_atom))),
      list(
        '{', $._impl_member, '}', choice($._newline, $.doc), $._comma_doc,
      ),
    ),

    _impl_member: $ => choice($.impl_member, $.arm),

    impl_member: $ => seq(
      optional('pub'),
      field('name', choice($.name, $.upper_name)),
      choice(
        seq(
          ':',
          field('type', $._annotation),
          optional(seq('=', field('value', $._member_value))),
        ),
        seq('=', field('value', $._member_value)),
      ),
    ),

    // member_value = arm | line: one arm, or a bracketed arm list.
    _member_value: $ => choice(alias($._one_arm, $.arms), $._element),

    _one_arm: $ => $.arm,

    // ---- Types -------------------------------------------------------

    // annotation = [ constraints, "=>", newlines ], type.
    _annotation: $ => choice($.qualified_type, $._type),

    qualified_type: $ => seq(
      $.constraint,
      repeat(seq(',', $.constraint)),
      '=>',
      field('type', $._type),
    ),

    // constraint = named_type, NAME | NAME, ":", type.
    constraint: $ => choice(
      seq(field('class', $.named_type), field('variable', $.name)),
      seq(field('member', $.name), ':', field('type', $._type)),
    ),

    // type = type_tuple, [ "->", newlines, type ]: right-associative.
    _type: $ => choice($.function_type, $._type_tuple),

    function_type: $ => prec.right(seq(
      field('parameter', $._type_tuple),
      '->',
      field('result', $._type),
    )),

    // `*`, `|`, `&` and `!`, each one flat node, `!` tightest.
    _type_tuple: $ => choice($.tuple_type, $._type_term),
    tuple_type: $ => seq($._type_term, repeat1(seq('*', $._type_term))),
    _type_term: $ => choice($.either_type, $._type_both),
    either_type: $ => seq($._type_both, repeat1(seq('|', $._type_both))),
    _type_both: $ => choice($.both_type, $._type_not),
    both_type: $ => seq($._type_not, repeat1(seq('&', $._type_not))),
    _type_not: $ => choice($.not_type, $._type_atom),
    not_type: $ => seq('!', $._type_atom),

    _type_atom: $ => choice(
      $.named_type,
      $.unit_type,
      $.array_type,
      $.labeled_type,
      seq('(', $._type, ')'),
    ),

    named_type: $ => seq(
      field('name', choice($.upper_name, $.name)),
      optional(field('arguments', $.type_arguments)),
    ),

    type_arguments: $ => list(
      '<', $._type_argument, '>', $._newline, $._comma,
    ),

    // type_argument = type | number_sum.
    _type_argument: $ => choice($._type, $.integer, $.number_sum),

    number_sum: $ => seq(
      $._number_part,
      repeat1(seq('+', $._number_part)),
    ),

    _number_part: $ => choice(
      $.integer, alias($._length_name, $.named_type),
    ),

    unit_type: _ => seq('(', ')'),

    array_type: $ => seq(
      '[',
      field('element', $._type),
      ';',
      field('length', choice(
        $.integer, $.number_sum, alias($._length_name, $.named_type),
      )),
      ']',
    ),

    _length_name: $ => field('name', $.name),

    labeled_type: $ => seq(
      '(',
      field('name', $.name),
      ':',
      field('type', $._type),
      optional(seq('=', field('default', $._operand))),
      ')',
    ),

    // ---- Lines -------------------------------------------------------

    // line = binding | scoped | pipeline. Where a comma separates items
    // an element line takes no arm list, which must be bracketed.
    _line: $ => choice($.binding, $.with, $._pipeline),
    _element: $ => choice(
      alias($._binding_element, $.binding),
      $.with,
      $._pipeline_element,
    ),

    // binding = pattern, [ ":", annotation ], "=", line.
    binding: $ => prec.right(seq(
      field('pattern', $._pattern),
      optional(seq(':', field('type', $._annotation))),
      '=',
      field('value', $._line),
    )),

    _binding_element: $ => prec.right(seq(
      field('pattern', $._pattern),
      optional(seq(':', field('type', $._annotation))),
      '=',
      field('value', $._element),
    )),

    // scoped = "with", NAME, operand, block.
    with: $ => seq(
      'with',
      field('key', $.name),
      field('value', $._head_operand),
      field('body', $.block),
    ),

    // pipeline = stage, { pipe, stage }: left, one level.
    _pipeline: $ => choice($.pipe, $._stage),
    pipe: $ => prec.left(PREC.pipe, seq(
      field('left', $._pipeline),
      field('operator', choice(...PIPES)),
      field('right', $._stage),
    )),

    _pipeline_element: $ => choice(
      alias($._pipe_element, $.pipe),
      $._stage_element,
    ),
    _pipe_element: $ => prec.left(PREC.pipe, seq(
      field('left', $._pipeline_element),
      field('operator', choice(...PIPES)),
      field('right', $._stage_element),
    )),

    // stage = arms | conditional | application | value.
    _stage: $ => choice($.arms, $._stage_element),
    _stage_element: $ => choice($.if, $.application, $._value),

    // application = operand, argument, { argument }.
    application: $ => seq(
      field('callee', $._operand),
      repeat1(field('argument', $._argument)),
    ),

    _argument: $ => choice($._operand, $.flag_argument),

    // argument = FLAG, [ "=", operand ], the `=` touching the flag.
    flag_argument: $ => seq(
      $.flag,
      optional(seq(token.immediate('='), field('value', $._operand))),
    ),

    // conditional = "if", stage, block, [ "else", ( conditional |
    // block ) ]. In an `if` or `with` head the first top-level `{` is
    // the body: a head operand is no record, block or braced arm list
    // (02-syntax.md §Lists and blocks, Heads).
    if: $ => seq(
      'if',
      field('condition', $._head_stage),
      field('consequence', $.block),
      optional(seq(
        'else',
        field('alternative', choice($.if, $.block)),
      )),
    ),


    // ---- Heads -------------------------------------------------------

    _head_stage: $ => choice(
      alias($._head_application, $.application),
      $._head_value,
    ),

    _head_application: $ => seq(
      field('callee', $._head_operand),
      repeat1(field('argument', choice($._head_operand, $.flag_argument))),
    ),

    ...values('head_', '_head_operand'),

    _head_operand: $ => choice(
      alias($._head_call, $.call),
      alias($._head_field, $.field),
      $._head_primary,
    ),

    _head_call: $ => prec(PREC.postfix, seq(
      field('callee', $._head_operand),
      $._call_list,
    )),

    _head_field: $ => prec(PREC.postfix, seq(
      field('value', $._head_operand),
      field('name', $.field_name),
    )),

    _head_primary: $ => choice(
      $.integer,
      $.float,
      $.string,
      $.raw_string,
      $.unit,
      $.name,
      $.upper_name,
      $.instance,
      $.word,
      $.path,
      $.selector,
      $._root,
      $.group,
      $.tuple,
      $.list,
      $.outcome,
      alias($._parenthesized_arms, $.arms),
    ),

    // ---- Values ------------------------------------------------------

    ...values('', '_operand'),

    // operand = primary, { call | field_access }: postfixes touch.
    _operand: $ => choice($.call, $.field, $._primary),

    call: $ => prec(PREC.postfix, seq(
      field('callee', $._operand),
      $._call_list,
    )),

    _call_list: $ => list(
      token.immediate(prec(1, '(')), $._call_argument, ')', $._newline,
      $._comma,
    ),

    _call_argument: $ => choice($._element, $.flag_argument),

    field: $ => prec(PREC.postfix, seq(
      field('value', $._operand),
      field('name', $.field_name),
    )),


    _primary: $ => choice(
      $.integer,
      $.float,
      $.string,
      $.raw_string,
      $.unit,
      $.name,
      $.upper_name,
      $.instance,
      $.word,
      $.path,
      $.selector,
      $._root,
      $.group,
      $.tuple,
      $.list,
      $.outcome,
      $.record,
      $.block,
      alias($._braced_arms, $.arms),
      alias($._parenthesized_arms, $.arms),
    ),

    // `/` is the root path at an operand position; division wins where
    // both read (02-syntax.md §Precedence).
    _root: $ => prec.dynamic(-1, alias('/', $.path)),

    unit: $ => seq('(', repeat($._newline), ')'),

    // instance = NAME, type_arguments, the `<` touching the name.
    instance: $ => seq(
      field('name', choice($.name, $.upper_name)),
      field('arguments', alias($._instance_arguments, $.type_arguments)),
    ),

    _instance_arguments: $ => list(
      token.immediate(prec(1, '<')), $._type_argument, '>', $._newline,
      $._comma,
    ),

    // group = "(", newlines, body, newlines, ")".
    group: $ => seq(
      '(',
      repeat($._newline),
      $._line,
      repeat(seq(repeat1($._newline), $._line)),
      repeat($._newline),
      ')',
    ),

    // tuple: a top-level comma before any `=>`; two parts or more.
    tuple: $ => seq(
      '(',
      repeat($._newline),
      $._element,
      $._comma,
      $._element,
      repeat(seq($._comma, $._element)),
      choice($._comma, repeat($._newline)),
      ')',
    ),

    list: $ => list('[', $._element, ']', $._newline, $._comma),

    // outcome = "[", ( ":" | "!" ), [ line ], "]": never a list.
    outcome: $ => prec(PREC.outcome, seq(
      '[',
      choice(':', $._outcome_bang),
      optional(field('payload', $._element)),
      ']',
    )),

    // The miss side's `!` after `[` is never a negation: touching, it
    // wins over `!`, so `[!x]` is an outcome and a negation in a list is
    // written `[(!x)]`.
    _outcome_bang: _ => alias(token.immediate(prec(1, '!')), '!'),

    // record = NAME, "{", field_values, "}" after an uppercase name.
    record: $ => prec(PREC.record, seq(
      field('name', $.upper_name),
      list('{', $.field_value, '}', $._newline, $._comma),
    )),

    field_value: $ => seq(
      field('name', $.name),
      optional(seq(':', field('value', $._element))),
    ),

    // block = "{", newlines, [ body ], newlines, "}".
    block: $ => seq(
      '{',
      repeat(choice($._newline, $.doc, seq($._line, $._newline))),
      optional($._line),
      '}',
    ),

    // A braced or parenthesized body whose first line has a top-level
    // `=>` is an arm list (02-syntax.md §Arms).
    _braced_arms: $ => prec.dynamic(1, seq(
      '{', repeat($._newline), $._arm_list, repeat($._newline), '}',
    )),
    _parenthesized_arms: $ => prec.dynamic(1, seq(
      '(', repeat($._newline), $._arm_list, repeat($._newline), ')',
    )),

    // ---- Arms --------------------------------------------------------

    // arms = arm, { ",", newlines, arm }, [ "," ]: a comma carries one
    // newline over.
    arms: $ => $._arm_list,

    _arm_list: $ => seq(
      $.arm,
      repeat(choice(
        seq(',', $.arm),
        prec.dynamic(1, seq(',', $._newline, $.arm)),
      )),
      optional(','),
    ),

    // arm = head, [ "if", stage ], "=>", newlines, line: the body, a
    // non-arm line, extends to the enclosing delimiter or comma.
    arm: $ => prec.right(seq(
      repeat1(field('pattern', $._head_pattern)),
      optional(seq('if', field('guard', $._stage_element))),
      '=>',
      field('body', $._element),
    )),

    // head_pattern = pattern | "(", NAME, "=", operand, ")".
    _head_pattern: $ => choice($._pattern, $.default_pattern),

    default_pattern: $ => seq(
      '(',
      field('name', $.name),
      '=',
      field('value', $._operand),
      ')',
    ),

    // ---- Patterns ----------------------------------------------------

    // pattern = pattern_atom | NAME, pattern: an uppercase name takes one
    // payload.
    _pattern: $ => choice($._pattern_atom, $.constructor_pattern),

    constructor_pattern: $ => prec.dynamic(1, seq(
      field('name', $.upper_name),
      field('payload', $._pattern),
    )),

    _pattern_atom: $ => choice(
      $.integer,
      $.float,
      $.string,
      $.raw_string,
      $.unit,
      $.name,
      $.upper_name,
      $.list_pattern,
      $.outcome_pattern,
      $.record_pattern,
      $.tuple_pattern,
      $.group_pattern,
    ),

    group_pattern: $ => seq('(', $._pattern, ')'),

    tuple_pattern: $ => seq(
      '(',
      repeat($._newline),
      $._pattern,
      $._comma,
      $._pattern,
      repeat(seq($._comma, $._pattern)),
      choice($._comma, repeat($._newline)),
      ')',
    ),

    list_pattern: $ => list('[', $._pattern, ']', $._newline, $._comma),

    outcome_pattern: $ => prec(PREC.outcome, seq(
      '[',
      choice(':', $._outcome_bang),
      optional(field('payload', $._pattern)),
      ']',
    )),

    record_pattern: $ => seq(
      field('name', $.upper_name),
      list(
        // The only word in a record pattern is `..`, the rest.
        '{', choice($.pattern_field, alias($.word, $.rest_pattern)), '}',
        $._newline,
        $._comma,
      ),
    ),

    // pattern_field = NAME, [ ":", pattern ] | WORD, the rest `..`.
    pattern_field: $ => seq(
      field('name', $.name),
      optional(seq(':', field('pattern', $._pattern))),
    ),


    // ---- Tokens ------------------------------------------------------

    // The regular lexer's fallback for runs the scanner leaves to it
    // (trailing backslashes), in classification order (01-lexical.md
    // §Word classification): every regex below matches a whole word or
    // less, `word` matches every other whole word, and a tie goes to the
    // earlier rule, so `word` is last.
    integer: _ => token(new RegExp(INTEGER)),
    float: _ => token(new RegExp(FLOAT)),
    flag: _ => token(new RegExp(String.raw`--?[A-Za-z]${BODY}*${TRAIL}`)),
    path: _ => token(
      new RegExp(String.raw`(?:\/|\.\/|\.\.\/)${BODY}*${TRAIL}`),
    ),
    selector: _ => token(new RegExp(SELECTOR)),
    name: _ => token(new RegExp(LOWER)),
    upper_name: _ => token(new RegExp(UPPER)),
    word: _ => token(new RegExp(WORD)),
  },
});
