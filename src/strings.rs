use std::borrow::Cow;

use crate::{
   ast::Literal,
   cursor::Cursor,
   dialect::Dialect,
   errors::Error,
   lexer::{
      Lexer,
      TokenKind,
   },
   span::Span,
};

/// Three double-quote characters delimiting multi-line string literals.
const TRIPLE_QUOTE: &str = "\"\"\"";

/// Result of decoding an escape sequence inside a string literal.
enum Escape {
   /// Decoded character produced by the escape sequence.
   Char(char),
   /// Whitespace escape that consumes following whitespace and newlines.
   Whitespace,
}

/// Delimiter configuration describing the quote style and hash count of a
/// string.
#[derive(Clone, Copy)]
pub struct Delimiter {
   /// Number of hashes in a raw string delimiter, or `None` for standard quoted
   /// strings.
   pub hashes:    Option<usize>,
   /// Indicates whether the string uses triple-quote multi-line syntax.
   pub multiline: bool,
}

#[expect(
   clippy::multiple_inherent_impl,
   reason = "string scanning lives beside its helpers"
)]
impl<'src> Lexer<'src> {
   /// Scans a double-quoted string literal or KDL v2 multi-line string.
   pub(crate) fn quoted(&mut self) -> Result<TokenKind<'src>, Error> {
      let start = self.cursor;
      let multiline = self.dialect() == Dialect::V2 && self.cursor.eat(TRIPLE_QUOTE);
      if !multiline {
         self.cursor.bump();
      }
      self.string_body(start, Delimiter {
         hashes: None,
         multiline,
      })
   }

   /// Scans a raw string literal or a hash-prefixed keyword like `#true` or
   /// `#null`.
   pub(crate) fn hash(&mut self) -> Result<TokenKind<'src>, Error> {
      let start = self.cursor;
      let hash_count = self.cursor.eat_while(|character| character == '#');
      if self.cursor.starts_with("\"") {
         let multiline = self.cursor.eat(TRIPLE_QUOTE);
         if !multiline {
            self.cursor.bump();
         }
         let delimiter = Delimiter {
            hashes: Some(hash_count),
            multiline,
         };
         return self.string_body(start, delimiter);
      }
      if hash_count != 1 {
         return Err(self.error_since(start, "unexpected `#`"));
      }
      let keywords = [
         ("#-inf", Literal::Decimal(Cow::Borrowed("#-inf"))),
         ("#inf", Literal::Decimal(Cow::Borrowed("#inf"))),
         ("#nan", Literal::Decimal(Cow::Borrowed("#nan"))),
         ("#true", Literal::Bool(true)),
         ("#false", Literal::Bool(false)),
         ("#null", Literal::Null),
      ];
      for (keyword_text, literal) in keywords {
         self.cursor = start;
         if !self.cursor.eat(keyword_text) {
            continue;
         }
         self.reject_trailing_identifier(start, "invalid keyword")?;
         return Ok(TokenKind::Literal(literal));
      }
      Err(Error::syntax(start.span_here(1), "unexpected `#`"))
   }

   /// Returns a cursor advanced past matching closing delimiters, or `None` if
   /// they do not match.
   #[inline]
   fn closing(&self, delimiter: Delimiter) -> Option<Cursor<'src>> {
      let mut probe = self.cursor;
      if probe.peek() != Some('"') {
         return None;
      }
      let quotes = if delimiter.multiline {
         TRIPLE_QUOTE
      } else {
         "\""
      };
      if !probe.eat(quotes) {
         return None;
      }
      for _ in 0..delimiter.hashes.unwrap_or(0) {
         if !probe.eat("#") {
            return None;
         }
      }
      Some(probe)
   }

   /// Scans the body of a string literal up to its closing delimiter and
   /// applies any dedenting.
   pub(crate) fn string_body(
      &mut self,
      start: Cursor<'src>,
      delimiter: Delimiter,
   ) -> Result<TokenKind<'src>, Error> {
      let dialect = self.dialect();
      if delimiter.multiline && !self.cursor.eat_newline() {
         let length = self.cursor.peek().map_or(0, char::len_utf8);
         self.fail(Error::syntax(
            self.cursor.span_here(length),
            "multi-line string must start with newline",
         ))?;
      }
      let content_start = self.cursor;
      let mut copy = Option::<String>::None;
      let mut escaped = false;
      let closing_finish = loop {
         if let Some(finish) = self.closing(delimiter) {
            break Some(finish);
         }
         let Some(character) = self.cursor.peek() else {
            self.fail(self.error_since(start, "unterminated string"))?;
            break None;
         };
         if character == '\\' && delimiter.hashes.is_none() {
            let escape_start = self.cursor;
            let text =
               copy.get_or_insert_with(|| escape_start.slice_from(content_start).to_owned());
            match self.escape() {
               Ok(Escape::Whitespace) => {},
               Ok(Escape::Char(decoded)) if !delimiter.multiline => text.push(decoded),
               Ok(Escape::Char(_)) => {
                  escaped = true;
                  text.push_str(self.cursor.slice_from(escape_start));
               },
               Err(error) => self.fail(error)?,
            }
            continue;
         }
         if !delimiter.multiline && dialect == Dialect::V2 && dialect.is_newline(character) {
            self.fail(self.error_since(start, "unterminated string"))?;
            break None;
         }
         if dialect.is_disallowed(character) {
            self.fail(self.error_at_char(character, "disallowed code point in string"))?;
            self.cursor.bump();
            continue;
         }
         self.cursor.bump();
         if let Some(text) = copy.as_mut() {
            text.push(character);
         }
      };
      let raw_text = self.cursor.slice_from(content_start);
      let value = if delimiter.multiline {
         let closing = self
            .cursor
            .span_here(closing_finish.map_or(0, |_| TRIPLE_QUOTE.len()));
         let dedent = copy.as_deref().map_or_else(
            || {
               MultilineBody {
                  text: raw_text,
                  closing,
               }
               .dedent()
            },
            |text| {
               MultilineBody { text, closing }
                  .dedent()
                  .map(|owned| Cow::Owned(owned.into_owned()))
            },
         );
         let dedented = match dedent {
            Ok(text) => text,
            Err(undented) => {
               self.fail(undented.error)?;
               Cow::Owned(undented.value)
            },
         };
         if escaped {
            Cow::Owned(MultilineBody::unescape(closing, &dedented)?)
         } else {
            dedented
         }
      } else {
         copy.map_or(Cow::Borrowed(raw_text), Cow::Owned)
      };
      if let Some(finish) = closing_finish {
         self.cursor = finish;
      }
      Ok(TokenKind::String(value))
   }

   /// Scans a backslash escape sequence and returns either a character or a
   /// whitespace skip.
   fn escape(&mut self) -> Result<Escape, Error> {
      let escape_start = self.cursor;
      self.cursor.bump();
      let Some(following) = self.cursor.peek() else {
         return Err(Error::syntax(
            escape_start.span_here(1),
            "unterminated string escape",
         ));
      };
      let dialect = self.dialect();
      if let Some(decoded) = dialect.decode_escape(following) {
         self.cursor.bump();
         return Ok(Escape::Char(decoded));
      }
      if following == 'u' {
         return self.unicode_escape(escape_start).map(Escape::Char);
      }
      if dialect == Dialect::V2
         && (dialect.is_space(following) || self.cursor.newline_len().is_some())
      {
         loop {
            if self.cursor.eat_newline() {
               continue;
            }
            match self.cursor.peek() {
               Some(filler) if dialect.is_space(filler) => {
                  self.cursor.bump();
               },
               _ => break,
            }
         }
         return Ok(Escape::Whitespace);
      }
      self.cursor.bump();
      Err(self.error_since(escape_start, "invalid string escape"))
   }

   /// Parses a `\u{...}` unicode escape sequence into a decoded character.
   fn unicode_escape(&mut self, escape_start: Cursor<'src>) -> Result<char, Error> {
      if !self.cursor.eat("u{") {
         return Err(Error::syntax(
            escape_start.span_here(1),
            "invalid unicode escape",
         ));
      }
      let digits_start = self.cursor;
      self
         .cursor
         .eat_while(|character| character.is_ascii_hexdigit());
      let hex_digits = self.cursor.slice_from(digits_start);
      match self.cursor.peek() {
         Some('}') => {},
         Some(other) => return Err(self.error_at_char(other, "invalid unicode escape")),
         None => return Err(self.error_since(escape_start, "unterminated unicode escape")),
      }
      self.cursor.bump();
      let invalid = || self.error_since(escape_start, "invalid unicode escape");
      if hex_digits.is_empty() || hex_digits.len() > 6 {
         return Err(invalid());
      }
      let value = hex_digits.chars().try_fold(0_u32, |value, character| {
         character
            .to_digit(16)
            .map(|digit| value * 16 + digit)
            .ok_or_else(invalid)
      })?;
      char::from_u32(value).ok_or_else(invalid)
   }
}

/// Multi-line string body processor handling indentation stripping per the KDL
/// v2 rules.
struct MultilineBody<'text> {
   /// Raw content slice between the opening and closing delimiters.
   text:    &'text str,
   /// Source span of the closing triple-quote delimiter for error reporting.
   closing: Span,
}

/// Recovered string value alongside a dedenting error for lenient parsing mode.
struct Undented {
   /// Syntax error describing the invalid multi-line closing or under-indented
   /// line.
   error: Error,
   /// Best-effort string content produced despite the dedenting failure.
   value: String,
}

/// Line segmentation and trailing indentation extracted from a multi-line
/// string body.
struct Lines<'text> {
   /// Individual content lines of the multi-line string without newline
   /// characters.
   content:         Vec<&'text str>,
   /// Full text slice of the lines preceding the final line containing the
   /// closing delimiter.
   body:            &'text str,
   /// Whitespace prefix on the final line defining the common indentation to
   /// strip.
   indent:          &'text str,
   /// Indicates whether every newline in the string body was a simple line
   /// feed.
   only_line_feeds: bool,
}

impl<'text> MultilineBody<'text> {
   /// Splits the body into content lines and isolates the baseline indentation.
   fn lines(&self) -> Lines<'text> {
      let mut content = Vec::new();
      let text_start = Cursor::new(self.text, Dialect::V2);
      let mut cursor = text_start;
      let mut line_start = cursor;
      let mut body = "";
      let mut only_line_feeds = true;
      while !cursor.at_end() {
         let line_end = cursor;
         if cursor.eat_newline() {
            only_line_feeds &= cursor.slice_from(line_end) == "\n";
            content.push(line_end.slice_from(line_start));
            body = line_end.slice_from(text_start);
            line_start = cursor;
            continue;
         }
         cursor.bump();
      }
      Lines {
         content,
         body,
         indent: cursor.slice_from(line_start),
         only_line_feeds,
      }
   }

   /// Strips common indentation from all lines, returning a recovered value on
   /// format errors.
   fn dedent(&self) -> Result<Cow<'text, str>, Undented> {
      let lines = self.lines();
      if !is_all_space(lines.indent) {
         return Err(Undented {
            error: Error::syntax(self.closing, "invalid multi-line string closing"),
            value: lines.content.join("\n"),
         });
      }
      if lines.indent.is_empty()
         && lines.only_line_feeds
         && !lines
            .content
            .iter()
            .any(|line| !line.is_empty() && is_all_space(line))
      {
         return Ok(Cow::Borrowed(lines.body));
      }
      let mut value = String::with_capacity(self.text.len());
      let mut under_indented = false;
      for (index, line) in lines.content.iter().enumerate() {
         if index > 0 {
            value.push('\n');
         }
         if is_all_space(line) {
            continue;
         }
         if let Some(rest) = line.strip_prefix(lines.indent) {
            value.push_str(rest);
         } else {
            under_indented = true;
            value.push_str(line.trim_start_matches(|character| Dialect::V2.is_space(character)));
         }
      }
      if under_indented {
         return Err(Undented {
            error: Error::syntax(self.closing, "multi-line string line under-indented"),
            value,
         });
      }
      Ok(Cow::Owned(value))
   }

   /// Resolves escape sequences across the previously dedented multi-line
   /// string body.
   fn unescape(closing: Span, dedented: &str) -> Result<String, Error> {
      let mut value = String::with_capacity(dedented.len());
      let mut lexer = Lexer::new(dedented, Dialect::V2);
      while let Some(character) = lexer.cursor.peek() {
         if character != '\\' {
            value.push(character);
            lexer.cursor.bump();
            continue;
         }
         match lexer.escape() {
            Ok(Escape::Char(decoded)) => value.push(decoded),
            Ok(Escape::Whitespace) => {},
            Err(_) => return Err(Error::syntax(closing, "invalid string escape")),
         }
      }
      Ok(value)
   }
}

/// Returns whether the given string slice consists entirely of KDL space
/// characters.
fn is_all_space(text: &str) -> bool {
   text
      .chars()
      .all(|character| Dialect::V2.is_space(character))
}
