use crate::{
   dialect::Dialect,
   span::Span,
};

/// A copyable scan position into the source text that only advances by whole
/// characters or matched prefixes.
#[derive(Clone, Copy)]
pub struct Cursor<'src> {
   /// Complete input string slice being scanned.
   source:   &'src str,
   /// Byte offset into `source`, always on a char boundary.
   position: usize,
   /// Dialect rules controlling newline and character classification.
   dialect:  Dialect,
}

impl<'src> Cursor<'src> {
   /// Creates a new cursor starting at the beginning of the given source text.
   pub(crate) const fn new(source: &'src str, dialect: Dialect) -> Self {
      Self {
         source,
         position: 0,
         dialect,
      }
   }

   /// Returns the dialect configuring this cursor.
   pub(crate) const fn dialect(self) -> Dialect {
      self.dialect
   }

   /// Returns the total byte length of the underlying source text.
   pub(crate) const fn source_len(self) -> usize {
      self.source.len()
   }

   /// Returns whether the cursor has reached or passed the end of the input.
   #[inline]
   pub(crate) const fn at_end(self) -> bool {
      self.position >= self.source.len()
   }

   /// Returns the remaining unconsumed slice of the source text from the
   /// current position.
   #[inline]
   #[expect(
      clippy::string_slice,
      reason = "position only moves by whole chars or matched prefixes"
   )]
   pub(crate) fn rest(self) -> &'src str {
      &self.source[self.position..]
   }

   /// Slices the source text from the earlier cursor position up to the current
   /// position.
   #[inline]
   #[expect(
      clippy::string_slice,
      reason = "position only moves by whole chars or matched prefixes"
   )]
   pub(crate) fn slice_from(self, start: Self) -> &'src str {
      &self.source[start.position..self.position]
   }

   /// Constructs a span covering the range from an earlier cursor position to
   /// this one.
   #[inline]
   pub(crate) const fn span_from(self, start: Self) -> Span {
      Span::new(start.position, self.position - start.position)
   }

   /// Constructs a span starting at the current position with the given byte
   /// length.
   #[inline]
   pub(crate) const fn span_here(self, length: usize) -> Span {
      Span::new(self.position, length)
   }

   /// Returns the next character without advancing the cursor, or `None` at the
   /// end of input.
   #[inline]
   pub(crate) fn peek(self) -> Option<char> {
      let byte = *self.source.as_bytes().get(self.position)?;
      if byte.is_ascii() {
         return Some(char::from(byte));
      }
      self.rest().chars().next()
   }

   /// Advances past and returns the next character, or returns `None` at the
   /// end of input.
   #[inline]
   pub(crate) fn bump(&mut self) -> Option<char> {
      let character = self.peek()?;
      self.position += character.len_utf8();
      Some(character)
   }

   /// Returns whether the remaining unconsumed input begins with the given
   /// prefix string.
   #[inline]
   pub(crate) fn starts_with(self, prefix: &str) -> bool {
      self.rest().starts_with(prefix)
   }

   /// Advances past the given prefix string if it matches, returning whether it
   /// matched.
   #[inline]
   pub(crate) fn eat(&mut self, prefix: &str) -> bool {
      let matched = self.starts_with(prefix);
      if matched {
         self.position += prefix.len();
      }
      matched
   }

   /// Advances past characters matching the predicate and returns how many were
   /// consumed.
   #[inline]
   pub(crate) fn eat_while(&mut self, mut accept: impl FnMut(char) -> bool) -> usize {
      let mut count = 0;
      while let Some(character) = self.peek()
         && accept(character)
      {
         self.position += character.len_utf8();
         count += 1;
      }
      count
   }

   /// Returns the byte length of a newline sequence at the current position, or
   /// `None` if absent.
   #[inline]
   pub(crate) fn newline_len(self) -> Option<usize> {
      let bytes = &self.source.as_bytes()[self.position..];
      match *bytes.first()? {
         b'\r' if bytes.get(1) == Some(&b'\n') => Some(2),
         b'\n' | b'\r' | 0x0C => Some(1),
         0x0B if self.dialect == Dialect::V2 => Some(1),
         byte if byte.is_ascii() => None,
         _ => {
            let character = self.peek()?;
            self
               .dialect
               .is_newline(character)
               .then(|| character.len_utf8())
         },
      }
   }

   /// Consumes a newline sequence at the current position, returning true if
   /// one was consumed.
   #[inline]
   pub(crate) fn eat_newline(&mut self) -> bool {
      match self.newline_len() {
         Some(length) => {
            self.position += length;
            true
         },
         None => false,
      }
   }
}
