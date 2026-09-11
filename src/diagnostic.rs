use std::{
   error::Error as StdError,
   fmt,
   iter::once,
};

use miette::{
   LabeledSpan,
   NamedSource,
   SourceCode,
   SourceSpan,
};

use crate::{
   errors::Error,
   span::Span,
};

#[derive(Debug)]
pub struct Diagnostic {
   /// Underlying error that supplies the span and message.
   error:   Error,
   /// Full source text with its file name so miette can render a snippet.
   source:  NamedSource<String>,
   /// Further diagnostics from the same source, reported after this one.
   related: Vec<Self>,
}

impl Diagnostic {
   #[inline]
   pub fn new<Name, Text>(error: Error, file_name: Name, source: Text) -> Self
   where
      Name: AsRef<str>,
      Text: Into<String>,
   {
      Self {
         error,
         source: NamedSource::new(file_name, source.into()),
         related: Vec::new(),
      }
   }

   #[inline]
   pub fn all<Errors, Name, Text>(errors: Errors, file_name: Name, source: Text) -> Option<Self>
   where
      Errors: IntoIterator<Item = Error>,
      Name: AsRef<str>,
      Text: Into<String>,
   {
      let text = source.into();
      let mut remaining = errors.into_iter();
      let mut first = Self::new(remaining.next()?, file_name.as_ref(), text.clone());
      first.related = remaining
         .map(|error| Self::new(error, file_name.as_ref(), text.clone()))
         .collect();
      Some(first)
   }

   #[must_use]
   #[inline]
   pub const fn error(&self) -> &Error {
      &self.error
   }

   #[must_use]
   #[inline]
   pub const fn span(&self) -> Span {
      self.error.span()
   }

   #[must_use]
   #[inline]
   pub fn into_error(self) -> Error {
      self.error
   }
}

impl fmt::Display for Diagnostic {
   #[inline]
   fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      fmt::Display::fmt(&self.error, f)
   }
}

impl StdError for Diagnostic {
   #[inline]
   fn source(&self) -> Option<&(dyn StdError + 'static)> {
      Some(&self.error)
   }
}

impl miette::Diagnostic for Diagnostic {
   #[inline]
   fn code<'src>(&'src self) -> Option<Box<dyn fmt::Display + 'src>> {
      Some(Box::new(self.error.kind().as_str()))
   }

   #[inline]
   fn labels(&self) -> Option<Box<dyn Iterator<Item = LabeledSpan> + '_>> {
      let span = SourceSpan::from((self.error.span().offset(), self.error.span().len()));
      let label = LabeledSpan::new_with_span(Some(self.error.message().to_owned()), span);
      Some(Box::new(once(label)))
   }

   #[inline]
   fn source_code(&self) -> Option<&dyn SourceCode> {
      Some(&self.source)
   }

   #[inline]
   fn related<'src>(
      &'src self,
   ) -> Option<Box<dyn Iterator<Item = &'src dyn miette::Diagnostic> + 'src>> {
      if self.related.is_empty() {
         return None;
      }
      Some(Box::new(
         self
            .related
            .iter()
            .map(|related| -> &dyn miette::Diagnostic { related }),
      ))
   }
}
