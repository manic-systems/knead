use crate::{
   ast::Document,
   errors::Error,
   parser,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Dialect {
   V1,
   #[default]
   V2,
}

impl Dialect {
   /// # Errors
   ///
   /// Fails at the first syntax error.
   #[inline]
   pub fn parse(self, source: &str) -> Result<Document<'_>, Error> {
      parser::parse(source, self)
   }

   #[must_use]
   #[inline]
   pub fn parse_lenient(self, source: &str) -> (Document<'_>, Vec<Error>) {
      parser::parse_lenient(source, self)
   }

   /// Reports whether `character` is horizontal whitespace, which includes the
   /// BOM only in v1.
   #[inline]
   pub(crate) const fn is_space(self, character: char) -> bool {
      matches!(
         character,
         '\u{0009}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{2000}'..='\u{200A}'
      ) || (matches!(self, Self::V1) && character == '\u{FEFF}')
   }

   /// Reports whether `character` ends a line, which includes vertical tab only
   /// in v2.
   #[inline]
   pub(crate) const fn is_newline(self, character: char) -> bool {
      matches!(
         character,
         '\u{000A}' | '\u{000D}' | '\u{0085}' | '\u{000C}' | '\u{2028}' | '\u{2029}'
      ) || (matches!(self, Self::V2) && character == '\u{000B}')
   }

   /// Reports whether `character` may never appear in a v2 document, and is
   /// always false in v1.
   #[inline]
   pub(crate) const fn is_disallowed(self, character: char) -> bool {
      matches!(self, Self::V2)
         && matches!(
            character,
            '\u{0000}'..='\u{0008}'
               | '\u{000E}'..='\u{001F}'
               | '\u{007F}'
               | '\u{200E}'
               | '\u{200F}'
               | '\u{202A}'..='\u{202E}'
               | '\u{2066}'..='\u{2069}'
               | '\u{FEFF}'
         )
   }

   /// Reports whether `character` may appear in a bare identifier, using a
   /// table for Latin-1.
   #[inline]
   pub(crate) fn is_identifier(self, character: char) -> bool {
      if let Ok(byte) = u8::try_from(character) {
         return match self {
            Self::V1 => V1_LATIN1_IDENTIFIER[usize::from(byte)],
            Self::V2 => V2_LATIN1_IDENTIFIER[usize::from(byte)],
         };
      }
      self.identifier_slow(character)
   }

   /// Classifies an identifier character without the ASCII table, so it can
   /// also build that table at compile time.
   const fn identifier_slow(self, character: char) -> bool {
      if self.is_space(character) || self.is_newline(character) || self.is_disallowed(character) {
         return false;
      }
      match self {
         Self::V1 => {
            !matches!(
               character,
               '\\' | '(' | ')' | '{' | '}' | '<' | '>' | ';' | '[' | ']' | '=' | ',' | '"'
            )
         },
         Self::V2 => {
            !matches!(
               character,
               '(' | ')' | '{' | '}' | '[' | ']' | '/' | '\\' | '"' | '#' | ';' | '='
            )
         },
      }
   }

   /// Maps the character after a backslash to the character it stands for, or
   /// `None` when the escape is not valid in this dialect.
   pub(crate) const fn decode_escape(self, character: char) -> Option<char> {
      match character {
         '"' => Some('"'),
         '\\' => Some('\\'),
         'b' => Some('\u{0008}'),
         'f' => Some('\u{000C}'),
         'n' => Some('\u{000A}'),
         'r' => Some('\u{000D}'),
         't' => Some('\u{0009}'),
         '/' if matches!(self, Self::V1) => Some('/'),
         's' if matches!(self, Self::V2) => Some('\u{0020}'),
         _ => None,
      }
   }

   /// Tells what the bare word `word` means in this dialect.
   pub(crate) fn keyword(self, word: &str) -> Keyword {
      match (self, word) {
         (Self::V1, "true") => Keyword::Bool(true),
         (Self::V1, "false") => Keyword::Bool(false),
         (Self::V1, "null") => Keyword::Null,
         (Self::V2, "true" | "false" | "null" | "inf" | "nan" | "-inf") => Keyword::Reserved,
         _ => Keyword::None,
      }
   }
}

pub enum Keyword {
   Bool(bool),
   Null,
   Reserved,
   None,
}

/// Builds the Latin-1 identifier lookup table for `dialect` at compile time.
#[expect(
   clippy::as_conversions,
   clippy::cast_possible_truncation,
   reason = "char::from is not const and the loop bound keeps index below 256"
)]
const fn identifier_table(dialect: Dialect) -> [bool; 256] {
   let mut table = [false; 256];
   let mut index = 0_usize;
   while index < 256 {
      table[index] = dialect.identifier_slow(index as u8 as char);
      index += 1;
   }
   table
}

/// Identifier lookup table for Latin-1 characters in v1.
static V1_LATIN1_IDENTIFIER: [bool; 256] = identifier_table(Dialect::V1);
/// Identifier lookup table for Latin-1 characters in v2.
static V2_LATIN1_IDENTIFIER: [bool; 256] = identifier_table(Dialect::V2);
