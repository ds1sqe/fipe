#![allow(dead_code)]

use crate::token::Token;
use crate::token::{self, Kind};
pub mod errors;

#[derive(Clone)]
pub struct Lexer {
    input: String,
    pos: usize,
    line_num: usize,
    next_pos: usize,
    cur: char,
}

#[derive(Debug, Clone)]
pub struct Position {
    pub pos: usize,
    pub line_num: usize,
}

impl Lexer {
    /// create new lexer from given input.
    ///
    /// * `input`: [String]
    pub fn new(input: String) -> Self {
        let mut lex = Lexer {
            input,
            pos: 0,
            line_num: 0,
            next_pos: 0,
            cur: '\0',
        };
        lex.read_char();
        lex
    }

    fn read_char(&mut self) {
        if self.next_pos >= self.input.len() {
            self.cur = '\0'
        } else {
            self.cur = self.input.chars().nth(self.next_pos).unwrap()
        }
        self.pos = self.next_pos;
        self.next_pos += 1;
    }

    fn peek_char(&self) -> char {
        if self.next_pos >= self.input.len() {
            return '\0';
        }
        self.input.chars().nth(self.next_pos).unwrap()
    }

    fn skip_whitespace(&mut self) {
        while self.cur.is_whitespace() {
            if self.cur == '\n' {
                self.line_num += 1;
            }
            self.read_char()
        }
    }

    fn read_id(&mut self) -> String {
        let start = self.pos;
        while !self.peek_char().is_whitespace()
            && self.peek_char().is_alphanumeric()
            || self.peek_char() == '_'
        {
            self.read_char();
        }
        self.input[start..self.pos + 1].to_string()
    }

    fn read_num(&mut self) -> Result<String, errors::LexerError> {
        let start = self.pos;
        while !self.peek_char().is_whitespace()
            && self.peek_char().is_alphanumeric()
        {
            if self.peek_char().is_alphabetic() {
                let err = errors::LexerError {
                    pos_start: start,
                    pos_end: self.pos,
                    reason: self.input[start..self.pos + 1].to_string()
                        + " is not a numeric",
                };
                return Err(err);
            }
            self.read_char();
        }
        Ok(self.input[start..self.pos + 1].to_string())
    }

    fn read_string(&mut self) -> Result<String, errors::LexerError> {
        let start = self.pos + 1;
        while self.peek_char() != '"' && self.peek_char() != '\0' {
            self.read_char();
        }

        if self.peek_char() == '\0' {
            let err = errors::LexerError {
                pos_start: start,
                pos_end: self.pos,
                reason: "closing (\") not found".to_string(),
            };
            return Err(err);
        }

        self.read_char();
        Ok(self.input[start..self.pos].to_string())
    }

    pub fn get_pos(&self) -> Position {
        Position {
            line_num: self.line_num,
            pos: self.pos,
        }
    }

    /// get next token
    pub fn next_token(&mut self) -> Token {
        self.skip_whitespace();

        let mut token: Token = Token::new(token::Kind::Illegal);

        match self.cur {
            '+' => token = Token::new(token::Kind::Plus),
            '-' => token = Token::new(token::Kind::Minus),
            '*' => token = Token::new(token::Kind::Product),
            '/' => token = Token::new(token::Kind::Divide),
            '%' => token = Token::new(token::Kind::Mod),
            '(' => token = Token::new(token::Kind::LPAREN),
            ')' => token = Token::new(token::Kind::RPAREN),
            '{' => token = Token::new(token::Kind::LBRACE),
            '}' => token = Token::new(token::Kind::RBRACE),
            '[' => token = Token::new(token::Kind::LBRACKET),
            ']' => token = Token::new(token::Kind::RBRACKET),
            ',' => token = Token::new(token::Kind::Comma),
            ';' => token = Token::new(token::Kind::Semicolon),
            '\0' => token = Token::new(token::Kind::EOF),
            '=' => {
                if self.peek_char() == '=' {
                    self.read_char();
                    token = Token::new(token::Kind::EQ);
                } else {
                    token = Token::new(token::Kind::Assign);
                }
            }
            '!' => {
                if self.peek_char() == '=' {
                    self.read_char();
                    token = Token::new(token::Kind::NOT_EQ);
                } else {
                    token = Token::new(token::Kind::Bang);
                }
            }
            '<' => {
                if self.peek_char() == '=' {
                    self.read_char();
                    token = Token::new(token::Kind::LT_OR_EQ);
                } else {
                    token = Token::new(token::Kind::LT);
                }
            }
            '>' => {
                if self.peek_char() == '=' {
                    self.read_char();
                    token = Token::new(token::Kind::GT_OR_EQ);
                } else {
                    token = Token::new(token::Kind::GT);
                }
            }
            '&' => {
                if self.peek_char() == '&' {
                    self.read_char();
                    token = Token::new(token::Kind::And);
                } else {
                    token = Token::new(token::Kind::Bit_And);
                }
            }
            '|' => {
                if self.peek_char() == '|' {
                    self.read_char();
                    token = Token::new(token::Kind::Or);
                } else {
                    token = Token::new(token::Kind::Bit_Or);
                }
            }
            '"' => {
                let result = self.read_string();
                if result.is_ok() {
                    token.kind = Kind::String;
                    token.literal = result.ok().unwrap();
                } else {
                    token.literal = result.err().unwrap().reason;
                }
            }

            any => {
                if any.is_alphabetic() {
                    token.literal = self.read_id();
                    token.kind = token::get_token_kind(&token.literal);
                } else if any.is_numeric() {
                    let read_result = self.read_num();
                    if read_result.is_ok() {
                        token.literal = read_result.unwrap();
                        token.kind = token::Kind::Int;
                    } else {
                        token.literal = read_result.err().unwrap().reason;
                    }
                }
            }
        }
        self.read_char();
        token
    }
}
