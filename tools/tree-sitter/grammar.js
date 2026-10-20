/**
 * @file Fipe grammar for tree-sitter
 * @author Kim Dae Up
 * @license MIT OR Apache-2.0
 *
 * Mirrors Documentation/spec/01-lexical.ebnf and 02-syntax.ebnf in the
 * language repository; the chapters there are normative. This file is
 * the lexical layer: every token of 01-lexical.md, and a flat `file`
 * rule over them. The scanner (src/scanner.c) owns what a regular
 * lexer cannot see; each regex below names its rule.
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
// `+ - * / % ++` are symbols only as whole words; `->` is matched before
// word scanning (01-lexical.md §Boundaries).
const WORD_SYMBOLS = ['+', '-', '*', '/', '%', '++', '->'];
const SYMBOLS = [
  '(', ')', '{', '}', '[', ']', ',', ';', ':', '=', '==', '!', '!=',
  '<', '<=', '>', '>=', '=>', '&',
];
// The five pipes: the scanner owns them, since the first LF after one
// is suppressed (01-lexical.md §Comments and logical lines).
const PIPES = ['|', '|:', '|!', '||', '&&'];

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
    // A run before a line join, which a regular lexer cannot end there.
    $.integer,
    $.float,
    $.flag,
    $.path,
    $.selector,
    $.name,
    $.word,
    '+', '-', '*', '/', '%', '++',
    ...KEYWORDS,
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

  rules: {
    file: $ => repeat($._token),

    _token: $ => choice(
      alias($._newline, $.newline),
      $.doc,
      $.inner_doc,
      $.string,
      $.raw_string,
      $.integer,
      $.float,
      $.flag,
      $.path,
      $.selector,
      $.name,
      $.word,
      ...KEYWORDS,
      ...WORD_SYMBOLS,
      ...SYMBOLS,
      ...PIPES,
    ),

    // Classification order (01-lexical.md §Word classification): every
    // regex below matches a whole word or less, `word` matches every
    // whole word, and a tie goes to the earlier rule, so `word` is last.
    integer: _ => token(new RegExp(INTEGER)),
    float: _ => token(new RegExp(FLOAT)),
    flag: _ => token(new RegExp(String.raw`--?[A-Za-z]${BODY}*${TRAIL}`)),
    path: _ => token(
      new RegExp(String.raw`(?:\/|\.\/|\.\.\/)${BODY}*${TRAIL}`),
    ),
    selector: _ => token(new RegExp(String.raw`\.${QUALIFIED}`)),
    name: _ => token(new RegExp(QUALIFIED)),
    word: _ => token(new RegExp(String.raw`${BODY}+${TRAIL}|\\+`)),
  },
});
