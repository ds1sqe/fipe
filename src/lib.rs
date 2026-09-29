#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

extern crate alloc;

pub mod ast;
pub mod decl;
pub mod embed;
mod frame;
pub mod infer;
pub mod lex;
pub mod parse;
pub mod resolve;
pub mod session;
pub mod token;
pub mod typed;
pub mod types;
