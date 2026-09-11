pub mod ast;
/// Character cursor over the source text with span bookkeeping.
mod cursor;
pub mod decode;
pub mod dialect;
pub mod errors;
/// Tokeniser that turns source text into a stream of tagged tokens.
mod lexer;
/// Recursive descent parser that builds the document from lexer tokens.
mod parser;
pub mod span;
/// Decoding of quoted, raw, and multiline string bodies.
mod strings;

#[cfg(feature = "miette")] pub mod diagnostic;

use crate::dialect::Dialect;

/// # Errors
///
/// Fails at the first syntax error.
#[inline]
pub fn parse(source: &str) -> Result<ast::Document<'_>, errors::Error> {
   Dialect::V2.parse(source)
}

#[must_use]
#[inline]
pub fn parse_lenient(source: &str) -> (ast::Document<'_>, Vec<errors::Error>) {
   Dialect::V2.parse_lenient(source)
}
