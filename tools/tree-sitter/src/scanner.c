// The external scanner of tree-sitter-fipe: the parts of
// Documentation/spec/01-lexical.md that a regular lexer cannot see.
//
// - Retained newline versus a suppressed one: the first LF after one of
//   the five pipes is not a NEWLINE when only spacing, a comment or a
//   doc stands between (§Comments and logical lines). The pipes are
//   scanned here so the state knows the last token was one.
// - The backslash line join, which ends a word: a run before it is
//   scanned and classified here, since a regex cannot look past the
//   backslash for the spacing and LF that make it a join.
// - Comments and docs: `#`, `##` and `##!`, with `###` a comment.
// - Quoted literals: a prefix is a whole word with no uppercase letter
//   that is not a keyword, a suffix a whole one-segment name that is not
//   a keyword, and a decoded quote crosses LF only by a join.
//
// Whether the previous token was a pipe: tokens of the regular lexer
// never cross a line and the scanner sees every LF, so the previous
// token is the last one scanned here exactly when the scan starts at
// the column where that one ended.

#include "tree_sitter/alloc.h"
#include "tree_sitter/parser.h"

#include <stdbool.h>
#include <stdint.h>
#include <string.h>

enum TokenType {
  NEWLINE,
  CONTINUATION,
  JOIN,
  COMMENT,
  DOC,
  INNER_DOC,
  STRING,
  RAW_STRING,
  PIPE,
  PIPE_GOOD,
  PIPE_BAD,
  OR,
  AND,
  INTEGER,
  FLOAT,
  FLAG,
  PATH,
  SELECTOR,
  NAME,
  WORD,
  PLUS,
  MINUS,
  STAR,
  SLASH,
  PERCENT,
  CONCAT,
  KW_IF,
  KW_ELSE,
  KW_WITH,
  KW_TYPE,
  KW_IMPL,
  KW_USE,
  KW_PUB,
};

typedef struct {
  // The last token scanned here was a pipe, or a comment or doc after
  // one, and it ended at `column`.
  bool continuation;
  uint32_t column;
  // The run being scanned; not part of the serialized state.
  int32_t *run;
  uint32_t length;
  uint32_t capacity;
} Scanner;

static void push(Scanner *s, int32_t c) {
  if (s->length == s->capacity) {
    s->capacity = s->capacity ? s->capacity * 2 : 64;
    s->run = ts_realloc(s->run, s->capacity * sizeof(int32_t));
  }
  s->run[s->length++] = c;
}

static bool spacing(int32_t c) {
  return c == '\t' || c == '\r' || c == ' ' || c == 0xA0 || c == 0x1680 ||
         (c >= 0x2000 && c <= 0x200A) || c == 0x2028 || c == 0x2029 ||
         c == 0x202F || c == 0x205F || c == 0x3000;
}

static bool control(int32_t c) {
  return (c >= 0 && c <= 0x1F) || (c >= 0x7F && c <= 0x9F);
}

static bool invisible(int32_t c) { return c == 0xFEFF || c == 0x200B; }

// §Boundaries: what ends a word.
static bool boundary(int32_t c) {
  if (spacing(c) || control(c) || invisible(c)) return true;
  switch (c) {
    case '(': case ')': case '{': case '}': case '[': case ']': case ',':
    case ';': case ':': case '=': case '<': case '>': case '!': case '|':
    case '&': case '"': case '\'': case '#':
      return true;
    default:
      return false;
  }
}

static bool letter(int32_t c) {
  return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z');
}

static bool digit(int32_t c) { return c >= '0' && c <= '9'; }

static bool name_start(int32_t c) { return letter(c) || c == '_'; }

static bool name_continue(int32_t c) { return name_start(c) || digit(c); }

static bool radix_digit(int32_t c, int radix) {
  switch (radix) {
    case 2: return c == '0' || c == '1';
    case 8: return c >= '0' && c <= '7';
    case 16:
      return digit(c) || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F');
    default: return digit(c);
  }
}

static bool same(const int32_t *w, uint32_t n, const char *text) {
  uint32_t i = 0;
  for (; text[i]; i++) {
    if (i >= n || w[i] != (unsigned char)text[i]) return false;
  }
  return i == n;
}

static bool starts(const int32_t *w, uint32_t n, const char *text) {
  for (uint32_t i = 0; text[i]; i++) {
    if (i >= n || w[i] != (unsigned char)text[i]) return false;
  }
  return true;
}

static const char *const KEYWORDS[] = {"if",   "else", "with", "type",
                                       "impl", "use",  "pub"};

static int keyword(const int32_t *w, uint32_t n) {
  for (int k = 0; k < 7; k++) {
    if (same(w, n, KEYWORDS[k])) return KW_IF + k;
  }
  return -1;
}

static bool identifier(const int32_t *w, uint32_t n) {
  if (n == 0 || !name_start(w[0])) return false;
  for (uint32_t i = 1; i < n; i++) {
    if (!name_continue(w[i])) return false;
  }
  return true;
}

// A qualified name: identifiers joined by single dots.
static bool qualified(const int32_t *w, uint32_t n) {
  uint32_t start = 0;
  for (uint32_t i = 0; i <= n; i++) {
    if (i == n || w[i] == '.') {
      if (!identifier(w + start, i - start)) return false;
      start = i + 1;
    }
  }
  return true;
}

// A quote's prefix: a name with no uppercase letter, not a keyword.
static bool prefix(const int32_t *w, uint32_t n) {
  if (n == 0 || !((w[0] >= 'a' && w[0] <= 'z') || w[0] == '_')) return false;
  for (uint32_t i = 0; i < n; i++) {
    int32_t c = w[i];
    if (!((c >= 'a' && c <= 'z') || digit(c) || c == '_')) return false;
  }
  return keyword(w, n) < 0;
}

// The end of the longest run of `radix` digits with single underscores
// between them.
static uint32_t digits_end(const int32_t *w, uint32_t n, int radix) {
  uint32_t end = 0;
  while (end < n && radix_digit(w[end], radix)) {
    end++;
    if (end + 1 < n && w[end] == '_' && radix_digit(w[end + 1], radix)) {
      end++;
    }
  }
  return end;
}

static bool suffix(const int32_t *w, uint32_t n) {
  return n > 0 && letter(w[0]) && identifier(w, n);
}

// §Numbers, read in order: a radix word, the longest decimal number, a
// program's prefix after a lone `0`, else a suffix. -1 is no number.
static int number(const int32_t *w, uint32_t n) {
  uint32_t sign = n > 0 && w[0] == '-' ? 1 : 0;
  const int32_t *body = w + sign;
  uint32_t length = n - sign;
  static const char *const RADIX[] = {"0x", "0o", "0b"};
  static const int BASE[] = {16, 8, 2};
  for (int r = 0; r < 3; r++) {
    if (starts(body, length, RADIX[r])) {
      uint32_t end = digits_end(body + 2, length - 2, BASE[r]);
      if (end == 0) return -1;
      uint32_t rest = 2 + end;
      if (rest == length || suffix(body + rest, length - rest)) {
        return INTEGER;
      }
      return -1;
    }
  }
  uint32_t end = digits_end(body, length, 10);
  if (end == 0) return -1;
  bool is_float = false;
  if (end < length && body[end] == '.') {
    uint32_t fraction = digits_end(body + end + 1, length - end - 1, 10);
    if (fraction > 0) {
      end += 1 + fraction;
      is_float = true;
    }
  }
  if (end < length && (body[end] == 'e' || body[end] == 'E')) {
    uint32_t at = end + 1;
    if (at < length && (body[at] == '+' || body[at] == '-')) at++;
    uint32_t exponent = digits_end(body + at, length - at, 10);
    if (exponent > 0) {
      end = at + exponent;
      is_float = true;
    }
  }
  int kind = is_float ? FLOAT : INTEGER;
  if (end == length) return kind;
  if (end == 1 && body[0] == '0') {
    int32_t p = body[1];
    if (p >= 'a' && p <= 'z' && p != 'x' && p != 'o' && p != 'b' &&
        length > 2) {
      bool alnum = true;
      for (uint32_t i = 2; i < length; i++) {
        if (!letter(body[i]) && !digit(body[i])) alnum = false;
      }
      if (alnum) return INTEGER;
    }
  }
  return suffix(body + end, length - end) ? kind : -1;
}

// §Word classification: the first matching row.
static int classify(const int32_t *w, uint32_t n) {
  int kind = number(w, n);
  if (kind >= 0) return kind;
  if (n == 1) {
    switch (w[0]) {
      case '+': return PLUS;
      case '-': return MINUS;
      case '*': return STAR;
      case '/': return SLASH;
      case '%': return PERCENT;
      default: break;
    }
  }
  if (same(w, n, "++")) return CONCAT;
  uint32_t dashes = starts(w, n, "--") ? 2 : starts(w, n, "-") ? 1 : 0;
  if (dashes > 0 && n > dashes && letter(w[dashes])) return FLAG;
  if (starts(w, n, "/") || starts(w, n, "./") || starts(w, n, "../")) {
    return PATH;
  }
  if (n > 1 && w[0] == '.' && qualified(w + 1, n - 1)) return SELECTOR;
  kind = keyword(w, n);
  if (kind >= 0) return kind;
  if (qualified(w, n)) return NAME;
  return WORD;
}

static void advance(TSLexer *lexer) { lexer->advance(lexer, false); }

static void skip(TSLexer *lexer) { lexer->advance(lexer, true); }

static bool accept(Scanner *s, TSLexer *lexer, int kind) {
  lexer->result_symbol = kind;
  s->continuation = false;
  return true;
}

// A comment, doc or pipe keeps a pipe's continuation; its end column
// identifies it as the previous token at the next scan.
static bool keep(Scanner *s, TSLexer *lexer, int kind, bool continuation) {
  lexer->result_symbol = kind;
  s->continuation = continuation;
  if (continuation) s->column = lexer->get_column(lexer);
  return true;
}

// A quote, its prefix already consumed, then a suffix touching it.
static bool quote(Scanner *s, TSLexer *lexer, const bool *valid) {
  int32_t mark = lexer->lookahead;
  bool raw = mark == '\'';
  int kind = raw ? RAW_STRING : STRING;
  if (!valid[kind]) return false;
  advance(lexer);
  for (;;) {
    if (lexer->eof(lexer)) return false;
    int32_t c = lexer->lookahead;
    if (c == mark) {
      advance(lexer);
      break;
    }
    if (!raw && c == '\n') return false;
    if (!raw && c == '\\') {
      advance(lexer);
      if (lexer->lookahead == '\r') {
        advance(lexer);
        if (lexer->lookahead != '\n') return false;
      }
      if (lexer->lookahead == '\n') {
        advance(lexer);
        while (spacing(lexer->lookahead)) advance(lexer);
        continue;
      }
      switch (lexer->lookahead) {
        case 'n': case 't': case 'r': case '\\': case '"': case '\'':
        case '0':
          advance(lexer);
          continue;
        case 'x':
          advance(lexer);
          for (int i = 0; i < 2; i++) {
            if (!radix_digit(lexer->lookahead, 16)) return false;
            advance(lexer);
          }
          continue;
        default:
          return false;
      }
    }
    if (raw && c == '\r') {
      advance(lexer);
      if (lexer->lookahead != '\n') return false;
      continue;
    }
    if (invisible(c) || (control(c) && c != '\t' && !(raw && c == '\n'))) {
      return false;
    }
    advance(lexer);
  }
  lexer->mark_end(lexer);
  // A suffix is a whole word. One ended by a line join is not seen as a
  // suffix: telling the join apart needs the scan past the backslash.
  s->length = 0;
  while (!lexer->eof(lexer) && name_continue(lexer->lookahead)) {
    push(s, lexer->lookahead);
    advance(lexer);
  }
  bool ended = lexer->eof(lexer) ||
               (boundary(lexer->lookahead) && lexer->lookahead != '\\');
  if (ended && suffix(s->run, s->length) && keyword(s->run, s->length) < 0) {
    lexer->mark_end(lexer);
  }
  return accept(s, lexer, kind);
}

// After a backslash: spacing then LF or EOF make it a line join. Moves
// past the spacing, and the LF when there is one.
static bool join_after(TSLexer *lexer) {
  while (spacing(lexer->lookahead)) advance(lexer);
  if (lexer->eof(lexer)) return true;
  if (lexer->lookahead == '\n') {
    advance(lexer);
    return true;
  }
  return false;
}

// A run of word characters. The regular lexer takes it unless a line
// join ends it, or it is a quote's prefix.
static bool run(Scanner *s, TSLexer *lexer, const bool *valid) {
  s->length = 0;
  for (;;) {
    if (lexer->eof(lexer)) break;
    int32_t c = lexer->lookahead;
    if (c == '\\') {
      lexer->mark_end(lexer);
      advance(lexer);
      int32_t next = lexer->lookahead;
      bool end = lexer->eof(lexer);
      if (!end && (next == '\\' || !boundary(next))) {
        push(s, '\\');
        continue;
      }
      if (end || next == '\n' || spacing(next)) {
        if (!join_after(lexer)) return false;
        if (s->length == 0) {
          lexer->mark_end(lexer);
          return valid[JOIN] && accept(s, lexer, JOIN);
        }
        int kind = classify(s->run, s->length);
        return valid[kind] && accept(s, lexer, kind);
      }
      return false;
    }
    if (boundary(c)) break;
    push(s, c);
    advance(lexer);
  }
  if (!lexer->eof(lexer) &&
      (lexer->lookahead == '"' || lexer->lookahead == '\'') &&
      prefix(s->run, s->length)) {
    return quote(s, lexer, valid);
  }
  return false;
}

void *tree_sitter_fipe_external_scanner_create(void) {
  return ts_calloc(1, sizeof(Scanner));
}

void tree_sitter_fipe_external_scanner_destroy(void *payload) {
  Scanner *s = payload;
  ts_free(s->run);
  ts_free(s);
}

unsigned tree_sitter_fipe_external_scanner_serialize(void *payload,
                                                     char *buffer) {
  Scanner *s = payload;
  buffer[0] = (char)s->continuation;
  memcpy(buffer + 1, &s->column, sizeof(s->column));
  return 1 + sizeof(s->column);
}

void tree_sitter_fipe_external_scanner_deserialize(void *payload,
                                                   const char *buffer,
                                                   unsigned length) {
  Scanner *s = payload;
  s->continuation = false;
  s->column = 0;
  if (length == 1 + sizeof(s->column)) {
    s->continuation = buffer[0];
    memcpy(&s->column, buffer + 1, sizeof(s->column));
  }
}

bool tree_sitter_fipe_external_scanner_scan(void *payload, TSLexer *lexer,
                                            const bool *valid) {
  Scanner *s = payload;
  bool continuation =
      s->continuation && lexer->get_column(lexer) == s->column;
  while (spacing(lexer->lookahead) || lexer->lookahead == 0xFEFF) {
    skip(lexer);
  }
  if (lexer->eof(lexer)) return false;
  int32_t c = lexer->lookahead;
  switch (c) {
    case '\n':
      advance(lexer);
      lexer->mark_end(lexer);
      if (continuation && valid[CONTINUATION]) {
        return accept(s, lexer, CONTINUATION);
      }
      return valid[NEWLINE] && accept(s, lexer, NEWLINE);
    case '#': {
      advance(lexer);
      int kind = COMMENT;
      if (lexer->lookahead == '#') {
        advance(lexer);
        if (lexer->lookahead != '#') {
          kind = DOC;
          if (lexer->lookahead == '!') {
            advance(lexer);
            kind = INNER_DOC;
          }
        }
      }
      while (!lexer->eof(lexer) && lexer->lookahead != '\n') advance(lexer);
      lexer->mark_end(lexer);
      return valid[kind] && keep(s, lexer, kind, continuation);
    }
    case '|': {
      advance(lexer);
      int kind = PIPE;
      if (lexer->lookahead == ':') kind = PIPE_GOOD;
      if (lexer->lookahead == '!') kind = PIPE_BAD;
      if (lexer->lookahead == '|') kind = OR;
      if (kind != PIPE) advance(lexer);
      lexer->mark_end(lexer);
      return valid[kind] && keep(s, lexer, kind, true);
    }
    case '&':
      advance(lexer);
      if (lexer->lookahead != '&') return false;
      advance(lexer);
      lexer->mark_end(lexer);
      return valid[AND] && keep(s, lexer, AND, true);
    case '"':
    case '\'':
      return quote(s, lexer, valid);
    default:
      if (c == '\\' || !boundary(c)) return run(s, lexer, valid);
      return false;
  }
}
