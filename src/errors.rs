use std::{
   error::Error as StdError,
   fmt,
};

use crate::span::Span;

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
   Syntax,
   Missing,
   Unexpected,
   Duplicate,
   Type,
   Conversion,
   Unsupported,
}

impl ErrorKind {
   #[must_use]
   #[inline]
   pub const fn as_str(self) -> &'static str {
      match self {
         Self::Syntax => "syntax error",
         Self::Missing => "missing value",
         Self::Unexpected => "unexpected value",
         Self::Duplicate => "duplicate value",
         Self::Type => "type mismatch",
         Self::Conversion => "conversion failure",
         Self::Unsupported => "unsupported value",
      }
   }
}

impl fmt::Display for ErrorKind {
   #[inline]
   fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      f.write_str(self.as_str())
   }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
   /// Source range the error points at.
   span:    Span,
   /// Category the error falls in, used for the display prefix.
   kind:    ErrorKind,
   /// Human readable detail shown after the kind.
   message: String,
}

impl Error {
   #[inline]
   pub fn new<Message>(kind: ErrorKind, span: Span, message: Message) -> Self
   where
      Message: Into<String>,
   {
      Self {
         span,
         kind,
         message: message.into(),
      }
   }

   #[inline]
   pub fn syntax<Message>(span: Span, message: Message) -> Self
   where
      Message: Into<String>,
   {
      Self::new(ErrorKind::Syntax, span, message)
   }

   #[inline]
   pub fn conversion<Cause>(span: Span, error: Cause) -> Self
   where
      Cause: fmt::Display,
   {
      Self::new(ErrorKind::Conversion, span, error.to_string())
   }

   #[must_use]
   #[inline]
   pub const fn span(&self) -> Span {
      self.span
   }

   #[must_use]
   #[inline]
   pub const fn kind(&self) -> ErrorKind {
      self.kind
   }

   #[must_use]
   #[inline]
   pub fn message(&self) -> &str {
      &self.message
   }
}

impl fmt::Display for Error {
   #[inline]
   fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      write!(
         f,
         "{} at byte offset {} ({})",
         self.kind.as_str(),
         self.span.offset(),
         self.message
      )
   }
}

impl StdError for Error {}
