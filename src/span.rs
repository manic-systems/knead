#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
   /// Byte offset where the range starts.
   offset: usize,
   /// Number of bytes covered by the range.
   length: usize,
}

impl Span {
   #[must_use]
   #[inline]
   pub const fn new(offset: usize, length: usize) -> Self {
      Self { offset, length }
   }

   #[must_use]
   #[inline]
   pub const fn offset(self) -> usize {
      self.offset
   }

   #[must_use]
   #[inline]
   pub const fn len(self) -> usize {
      self.length
   }

   #[must_use]
   #[inline]
   pub const fn is_empty(self) -> bool {
      self.length == 0
   }

   #[must_use]
   #[inline]
   pub const fn end(self) -> usize {
      self.offset + self.length
   }

   #[must_use]
   #[inline]
   pub fn join(self, other: Self) -> Self {
      let offset = self.offset.min(other.offset);
      Self::new(offset, self.end().max(other.end()) - offset)
   }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spanned<Data> {
   pub value: Data,
   pub span:  Span,
}

impl<Data> Spanned<Data> {
   #[inline]
   pub const fn new(value: Data, span: Span) -> Self {
      Self { value, span }
   }

   #[inline]
   pub const fn span(&self) -> Span {
      self.span
   }
}

impl<Text: AsRef<str>> Spanned<Text> {
   #[inline]
   pub fn value(&self) -> &str {
      self.value.as_ref()
   }
}
