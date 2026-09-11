use std::borrow::Cow;

use crate::{
   ast::{
      Integer,
      Literal,
      Radix,
   },
   cursor::Cursor,
   dialect::{
      Dialect,
      Keyword,
   },
   errors::Error,
   span::Span,
   strings::Delimiter,
};

/// Lexical token paired with its source span.
#[derive(Clone, Debug, PartialEq)]
pub struct Token<'src> {
   /// Specific lexical category and payload of the token.
   pub kind: TokenKind<'src>,
   /// Byte offset and length of this token in the source text.
   pub span: Span,
}

/// Lexical token classification produced by the tokenizer.
#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind<'src> {
   /// Horizontal whitespace or comments that separate tokens.
   Space,
   /// Line terminator or line-ending comment ending a node or statement.
   Newline,
   /// Quoted or raw string literal holding unescaped text.
   String(Cow<'src, str>),
   /// Bare word identifier naming a node, property, or type annotation.
   Identifier(Cow<'src, str>),
   /// Parsed literal value such as a boolean, number, or null.
   Literal(Literal<'src>),
   /// Opening parenthesis beginning a type annotation.
   OpenParen,
   /// Closing parenthesis ending a type annotation.
   CloseParen,
   /// Opening brace beginning a child node block.
   OpenBrace,
   /// Closing brace ending a child node block.
   CloseBrace,
   /// Equals sign separating a property key from its value.
   Equals,
   /// Explicit node terminator symbol.
   Semicolon,
   /// Comment prefix that comments out the following node or argument.
   Slashdash,
   /// Marker indicating the end of the input source text.
   Eof,
}

/// Discriminant tag identifying token variants without carrying data payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
   /// Discriminant identifying whitespace or horizontal separation.
   Space,
   /// Discriminant identifying line endings and statement terminators.
   Newline,
   /// Discriminant identifying quoted or raw text literals.
   String,
   /// Discriminant identifying bare identifiers and names.
   Identifier,
   /// Discriminant identifying scalar literal values.
   Literal,
   /// Discriminant identifying the start of a type annotation.
   OpenParen,
   /// Discriminant identifying the end of a type annotation.
   CloseParen,
   /// Discriminant identifying the start of a child node block.
   OpenBrace,
   /// Discriminant identifying the end of a child node block.
   CloseBrace,
   /// Discriminant identifying property assignment symbols.
   Equals,
   /// Discriminant identifying explicit node terminator symbols.
   Semicolon,
   /// Discriminant identifying node or argument exclusion prefixes.
   Slashdash,
   /// Discriminant identifying the end of the input stream.
   Eof,
}

impl TokenKind<'_> {
   /// Returns the discriminant tag corresponding to this token kind.
   pub(crate) const fn tag(&self) -> Tag {
      match *self {
         TokenKind::Space => Tag::Space,
         TokenKind::Newline => Tag::Newline,
         TokenKind::String(_) => Tag::String,
         TokenKind::Identifier(_) => Tag::Identifier,
         TokenKind::Literal(_) => Tag::Literal,
         TokenKind::OpenParen => Tag::OpenParen,
         TokenKind::CloseParen => Tag::CloseParen,
         TokenKind::OpenBrace => Tag::OpenBrace,
         TokenKind::CloseBrace => Tag::CloseBrace,
         TokenKind::Equals => Tag::Equals,
         TokenKind::Semicolon => Tag::Semicolon,
         TokenKind::Slashdash => Tag::Slashdash,
         TokenKind::Eof => Tag::Eof,
      }
   }
}

/// Token scanner that produces lexical tokens and records diagnostics in
/// lenient mode.
pub struct Lexer<'src> {
   /// Current position and source slice being scanned.
   pub cursor:  Cursor<'src>,
   /// Diagnostics accumulated during lenient parsing mode.
   pub errors:  Vec<Error>,
   /// Controls whether lexical errors are accumulated rather than halting
   /// immediately.
   pub lenient: bool,
}

impl<'src> Lexer<'src> {
   /// Creates a strict lexer that stops on the first encountered syntax error.
   pub(crate) fn new(source: &'src str, dialect: Dialect) -> Self {
      Self::with_mode(source, dialect, false)
   }

   /// Creates a lenient lexer that records syntax errors and attempts recovery.
   pub(crate) fn lenient(source: &'src str, dialect: Dialect) -> Self {
      Self::with_mode(source, dialect, true)
   }

   /// Initializes a cursor, consumes any leading byte order mark, and sets
   /// error tolerance.
   fn with_mode(source: &'src str, dialect: Dialect, lenient: bool) -> Self {
      let mut cursor = Cursor::new(source, dialect);
      cursor.eat("\u{FEFF}");
      Self {
         cursor,
         errors: Vec::new(),
         lenient,
      }
   }

   /// Returns whether the lexer is configured to recover from syntax errors.
   pub(crate) const fn is_lenient(&self) -> bool {
      self.lenient
   }

   /// Returns the KDL dialect rules applied by this lexer.
   pub(crate) const fn dialect(&self) -> Dialect {
      self.cursor.dialect()
   }

   /// Returns the total byte length of the input source text.
   pub(crate) const fn source_len(&self) -> usize {
      self.cursor.source_len()
   }

   /// Records the error in lenient mode or returns it directly in strict mode.
   pub(crate) fn fail(&mut self, error: Error) -> Result<(), Error> {
      if !self.lenient {
         return Err(error);
      }
      self.errors.push(error);
      Ok(())
   }

   /// Returns the next lexical token, skipping errors when operating in lenient
   /// mode.
   pub(crate) fn next_token(&mut self) -> Result<Token<'src>, Error> {
      loop {
         let start = self.cursor;
         let Some(current) = self.cursor.peek() else {
            return Ok(Token {
               kind: TokenKind::Eof,
               span: start.span_here(0),
            });
         };
         match self.token(current, start) {
            Ok(kind) => {
               return Ok(Token {
                  kind,
                  span: self.cursor.span_from(start),
               });
            },
            Err(error) => {
               self.fail(error)?;
               if self.cursor.span_from(start).is_empty() {
                  self.cursor.bump();
               }
            },
         }
      }
   }

   /// Dispatches the current character to scan comments, symbols, strings, or
   /// numbers.
   fn token(&mut self, current: char, start: Cursor<'src>) -> Result<TokenKind<'src>, Error> {
      let dialect = self.dialect();
      if dialect.is_disallowed(current) {
         return Err(self.error_at_char(current, "disallowed code point"));
      }
      let kind = match current {
         '/' => {
            if self.cursor.eat("//") {
               self.line_comment()?;
               TokenKind::Newline
            } else if self.cursor.eat("/*") {
               self.block_comment(start)?;
               TokenKind::Space
            } else if self.cursor.eat("/-") {
               TokenKind::Slashdash
            } else {
               return Err(Error::syntax(start.span_here(1), "unexpected `/`"));
            }
         },
         '\\' => {
            self.cursor.bump();
            self.escline(start)?;
            TokenKind::Space
         },
         '(' => self.single(TokenKind::OpenParen),
         ')' => self.single(TokenKind::CloseParen),
         '{' => self.single(TokenKind::OpenBrace),
         '}' => self.single(TokenKind::CloseBrace),
         '=' => self.single(TokenKind::Equals),
         ';' => self.single(TokenKind::Semicolon),
         '"' => self.quoted()?,
         '#' if dialect == Dialect::V2 => self.hash()?,
         _ if dialect.is_space(current) => {
            self
               .cursor
               .eat_while(|character| dialect.is_space(character));
            TokenKind::Space
         },
         _ if self.cursor.eat_newline() => TokenKind::Newline,
         _ if dialect.is_identifier(current) => self.word()?,
         _ => return Err(self.error_at_char(current, "unexpected character")),
      };
      Ok(kind)
   }

   /// Consumes one single-byte punctuation character and yields the token kind.
   fn single(&mut self, kind: TokenKind<'src>) -> TokenKind<'src> {
      self.cursor.bump();
      kind
   }

   /// Builds a syntax error spanning the UTF-8 byte sequence of the given
   /// character.
   pub(crate) fn error_at_char(&self, character: char, message: &str) -> Error {
      Error::syntax(self.cursor.span_here(character.len_utf8()), message)
   }

   /// Builds a syntax error spanning the range from a previous position to the
   /// current one.
   pub(crate) fn error_since(&self, start: Cursor<'src>, message: &str) -> Error {
      Error::syntax(self.cursor.span_from(start), message)
   }

   /// Consumes a single-line comment up to the newline while rejecting
   /// disallowed characters.
   fn line_comment(&mut self) -> Result<(), Error> {
      while let Some(character) = self.cursor.peek() {
         if self.cursor.eat_newline() {
            return Ok(());
         }
         if self.dialect().is_disallowed(character) {
            return Err(self.error_at_char(character, "disallowed code point in comment"));
         }
         self.cursor.bump();
      }
      Ok(())
   }

   /// Consumes nested block comments, returning an error on unterminated
   /// blocks.
   fn block_comment(&mut self, start: Cursor<'src>) -> Result<(), Error> {
      let mut depth = 1_u32;
      while let Some(character) = self.cursor.peek() {
         if self.cursor.eat("/*") {
            depth += 1;
            continue;
         }
         if self.cursor.eat("*/") {
            depth -= 1;
            if depth == 0 {
               return Ok(());
            }
            continue;
         }
         if self.dialect().is_disallowed(character) {
            return Err(self.error_at_char(character, "disallowed code point in comment"));
         }
         self.cursor.bump();
      }
      Err(self.error_since(start, "unterminated block comment"))
   }

   /// Consumes an escaped newline continuation along with any trailing
   /// whitespace or comments.
   fn escline(&mut self, start: Cursor<'src>) -> Result<(), Error> {
      loop {
         let comment_start = self.cursor;
         if self.cursor.eat("/*") {
            self.block_comment(comment_start)?;
            continue;
         }
         match self.cursor.peek() {
            Some(character) if self.dialect().is_space(character) => {
               self.cursor.bump();
            },
            _ => break,
         }
      }
      if self.cursor.eat("//") {
         return self.line_comment();
      }
      if self.cursor.eat_newline() {
         return Ok(());
      }
      match self.cursor.bump() {
         None => Ok(()),
         Some(_) => Err(self.error_since(start, "invalid line continuation")),
      }
   }

   /// Scans an identifier, keyword, raw string, or numeric literal.
   fn word(&mut self) -> Result<TokenKind<'src>, Error> {
      let dialect = self.dialect();
      let start = self.cursor;
      if dialect == Dialect::V1
         && self.cursor.eat("r")
         && let hashes = self.cursor.eat_while(|character| character == '#')
         && self.cursor.eat("\"")
      {
         let delimiter = Delimiter {
            hashes:    Some(hashes),
            multiline: false,
         };
         return self.string_body(start, delimiter);
      }
      self.cursor = start;
      let signed = matches!(self.cursor.peek(), Some('+' | '-'));
      if signed {
         self.cursor.bump();
      }
      if let Some(radix) = self.radix_prefix() {
         return self.radix_number(start, signed, radix);
      }
      if matches!(self.cursor.peek(), Some(digit) if digit.is_ascii_digit()) {
         return self.decimal_number(start);
      }
      self.cursor = start;
      loop {
         self
            .cursor
            .eat_while(|character| dialect.is_identifier(character));
         match self.cursor.peek() {
            Some(character) if self.lenient && dialect.is_disallowed(character) => {
               self.fail(self.error_at_char(character, "disallowed code point"))?;
               self.cursor.bump();
            },
            _ => break,
         }
      }
      let word = self.cursor.slice_from(start);
      let mut letters = word.chars();
      let Some(first_letter) = letters.next() else {
         return Err(Error::syntax(start.span_here(1), "unexpected character"));
      };
      match dialect.keyword(word) {
         Keyword::Bool(flag) => return Ok(TokenKind::Literal(Literal::Bool(flag))),
         Keyword::Null => return Ok(TokenKind::Literal(Literal::Null)),
         Keyword::Reserved => {
            return Err(self.error_since(start, "bare keyword must use `#` prefix"));
         },
         Keyword::None => {},
      }
      let second_letter = letters.next();
      let third_letter = letters.next();
      let looks_numeric = dialect == Dialect::V2
         && match first_letter {
            '+' | '-' => {
               match second_letter {
                  Some(digit) if digit.is_ascii_digit() => true,
                  Some('.') => third_letter.is_some_and(|letter| letter.is_ascii_digit()),
                  _ => false,
               }
            },
            '.' => second_letter.is_some_and(|letter| letter.is_ascii_digit()),
            _ => false,
         };
      if looks_numeric {
         return Err(self.error_since(start, "invalid identifier"));
      }
      Ok(TokenKind::Identifier(Cow::Borrowed(word)))
   }

   /// Recognizes and consumes a hexadecimal, octal, or binary base prefix.
   fn radix_prefix(&mut self) -> Option<Radix> {
      if self.cursor.eat("0x") {
         Some(Radix::Hexadecimal)
      } else if self.cursor.eat("0o") {
         Some(Radix::Octal)
      } else if self.cursor.eat("0b") {
         Some(Radix::Binary)
      } else {
         None
      }
   }

   /// Scans a non-decimal integer with digits conforming to the given radix.
   fn radix_number(
      &mut self,
      start: Cursor<'src>,
      signed: bool,
      radix: Radix,
   ) -> Result<TokenKind<'src>, Error> {
      let digits_start = self.cursor;
      if !matches!(self.cursor.peek(), Some(digit) if radix.accepts(digit)) {
         let dialect = self.dialect();
         self
            .cursor
            .eat_while(|character| dialect.is_identifier(character));
         return Err(self.error_since(start, "invalid number"));
      }
      self
         .cursor
         .eat_while(|character| character == '_' || radix.accepts(character));
      self.reject_trailing_identifier(start, "invalid number")?;
      let raw_digits = self.cursor.slice_from(digits_start);
      let digits = if signed {
         let mut owned = String::with_capacity(raw_digits.len() + 1);
         owned.push_str(digits_start.slice_from(start));
         owned.extend(raw_digits.chars().filter(|character| *character != '_'));
         Cow::Owned(owned)
      } else {
         strip_underscores(raw_digits)
      };
      Ok(TokenKind::Literal(Literal::Integer(Integer {
         radix,
         digits,
      })))
   }

   /// Scans a decimal integer or floating-point number including optional
   /// exponents.
   fn decimal_number(&mut self, start: Cursor<'src>) -> Result<TokenKind<'src>, Error> {
      self.cursor.eat_while(is_digit_or_underscore);
      let mut fraction = self.cursor;
      let dotted =
         fraction.eat(".") && matches!(fraction.peek(), Some(digit) if digit.is_ascii_digit());
      if dotted {
         match self.dialect() {
            Dialect::V1 => fraction.eat_while(|character| character.is_ascii_digit()),
            Dialect::V2 => fraction.eat_while(is_digit_or_underscore),
         };
         self.cursor = fraction;
      }
      let mut exponent = self.cursor;
      let mut has_exponent = false;
      if exponent.eat("e") || exponent.eat("E") {
         if matches!(exponent.peek(), Some('+' | '-')) {
            exponent.bump();
         }
         if matches!(exponent.peek(), Some(digit) if digit.is_ascii_digit()) {
            exponent.eat_while(is_digit_or_underscore);
            self.cursor = exponent;
            has_exponent = true;
         }
      }
      self.reject_trailing_identifier(start, "invalid number")?;
      let cleaned = strip_underscores(self.cursor.slice_from(start));
      if dotted || has_exponent {
         return Ok(TokenKind::Literal(Literal::Decimal(cleaned)));
      }
      Ok(TokenKind::Literal(Literal::Integer(Integer {
         radix:  Radix::Decimal,
         digits: cleaned,
      })))
   }

   /// Ensures a parsed literal is not immediately followed by illegal
   /// identifier characters.
   pub(crate) fn reject_trailing_identifier(
      &mut self,
      start: Cursor<'src>,
      message: &str,
   ) -> Result<(), Error> {
      match self.cursor.peek() {
         Some(following) if self.dialect().is_identifier(following) => {
            let dialect = self.dialect();
            self
               .cursor
               .eat_while(|character| dialect.is_identifier(character));
            Err(self.error_since(start, message))
         },
         _ => Ok(()),
      }
   }
}

/// Removes formatting underscores from number literals, borrowing when none are
/// present.
fn strip_underscores(text: &str) -> Cow<'_, str> {
   if text.contains('_') {
      Cow::Owned(text.chars().filter(|character| *character != '_').collect())
   } else {
      Cow::Borrowed(text)
   }
}

/// Checks if a character is an ASCII digit or an underscore.
const fn is_digit_or_underscore(character: char) -> bool {
   character.is_ascii_digit() || character == '_'
}
