use std::{
   borrow::Cow,
   fmt::{
      Display,
      Error as FormatError,
      Formatter,
      Result as FormatResult,
   },
   str::FromStr,
};

use crate::{
   errors::{
      Error,
      ErrorKind,
   },
   span::{
      Span,
      Spanned,
   },
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document<'src> {
   pub nodes: Vec<Node<'src>>,
   pub span:  Span,
}

impl<'src> Document<'src> {
   #[must_use]
   #[inline]
   pub fn nodes(&self) -> &[Node<'src>] {
      &self.nodes
   }

   #[must_use]
   #[inline]
   pub const fn span(&self) -> Span {
      self.span
   }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node<'src> {
   pub name:       Spanned<Cow<'src, str>>,
   pub type_name:  Option<Spanned<Cow<'src, str>>>,
   pub arguments:  Vec<Value<'src>>,
   pub properties: Vec<Property<'src>>,
   pub children:   Option<Document<'src>>,
   pub span:       Span,
}

impl<'src> Node<'src> {
   #[must_use]
   #[inline]
   pub const fn name(&self) -> &Spanned<Cow<'src, str>> {
      &self.name
   }

   #[must_use]
   #[inline]
   pub const fn span(&self) -> Span {
      self.span
   }

   #[must_use]
   #[inline]
   pub const fn children(&self) -> Option<&Document<'src>> {
      self.children.as_ref()
   }

   #[inline]
   pub fn child_nodes(&self) -> &[Self] {
      self.children.as_ref().map_or(&[], Document::nodes)
   }

   /// # Errors
   ///
   /// Fails when the node carries a type annotation.
   #[inline]
   pub fn reject_type(&self) -> Result<(), Error> {
      self.type_name.as_ref().map_or(Ok(()), |found| {
         Err(Error::new(
            ErrorKind::Unexpected,
            found.span,
            format!("unexpected type annotation {:?}", found.value),
         ))
      })
   }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Property<'src> {
   pub name:  Spanned<Cow<'src, str>>,
   pub value: Value<'src>,
   pub span:  Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Value<'src> {
   pub type_name: Option<Spanned<Cow<'src, str>>>,
   pub literal:   Literal<'src>,
   pub span:      Span,
}

impl Value<'_> {
   /// # Errors
   ///
   /// Fails when the value carries a type annotation.
   #[inline]
   pub fn reject_type(&self) -> Result<(), Error> {
      self.type_name.as_ref().map_or(Ok(()), |found| {
         Err(Error::new(
            ErrorKind::Type,
            found.span,
            format!("expected no type annotation but found {:?}", found.value),
         ))
      })
   }

   /// # Errors
   ///
   /// Fails when the value carries a type annotation that is not in `accepted`.
   #[inline]
   pub fn expect_type(&self, accepted: &[&str]) -> Result<(), Error> {
      match self.type_name.as_ref() {
         Some(found) if !accepted.contains(&&*found.value) => {
            Err(Error::new(ErrorKind::Type, found.span, match *accepted {
               [want] => format!("expected type {:?} but found {:?}", want, found.value),
               _ => format!("expected no type annotation but found {:?}", found.value),
            }))
         },
         _ => Ok(()),
      }
   }

   /// # Errors
   ///
   /// Fails when the literal is not a string.
   #[inline]
   pub fn expect_string(&self) -> Result<&str, Error> {
      match self.literal {
         Literal::String(ref text) => Ok(text),
         Literal::Integer(_) | Literal::Decimal(_) | Literal::Bool(_) | Literal::Null => {
            Err(self.type_mismatch("string"))
         },
      }
   }

   /// # Errors
   ///
   /// Fails when the value is annotated, is not a string, or does not parse as
   /// `Item`.
   #[inline]
   pub fn parse_str<Item>(&self) -> Result<Item, Error>
   where
      Item: FromStr,
      Item::Err: Display,
   {
      self.reject_type()?;
      self
         .expect_string()?
         .parse()
         .map_err(|error| Error::conversion(self.span, error))
   }

   #[must_use]
   #[inline]
   pub fn type_mismatch(&self, expected: &str) -> Error {
      Error::new(
         ErrorKind::Type,
         self.span,
         format!(
            "expected {expected} scalar but found {}",
            self.literal.kind()
         ),
      )
   }
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Literal<'src> {
   String(Cow<'src, str>),
   Integer(Integer<'src>),
   Decimal(Cow<'src, str>),
   Bool(bool),
   Null,
}

impl Literal<'_> {
   #[must_use]
   #[inline]
   pub const fn kind(&self) -> &'static str {
      match *self {
         Literal::String(_) => "string",
         Literal::Integer(_) => "integer",
         Literal::Decimal(_) => "decimal",
         Literal::Bool(_) => "boolean",
         Literal::Null => "null",
      }
   }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Integer<'src> {
   pub radix:  Radix,
   pub digits: Cow<'src, str>,
}

impl Display for Integer<'_> {
   #[inline]
   fn fmt(&self, f: &mut Formatter<'_>) -> FormatResult {
      let negative = self.digits.starts_with('-');
      let magnitude = self.digits.trim_start_matches(['-', '+']);
      let mut decimal = vec![0_u32];
      let radix = u32::from(self.radix);
      for character in magnitude.chars() {
         let mut carry = character.to_digit(radix).ok_or(FormatError)?;
         for digit in &mut decimal {
            let product = *digit * radix + carry;
            *digit = product % 10;
            carry = product / 10;
         }
         while carry > 0 {
            decimal.push(carry % 10);
            carry /= 10;
         }
      }
      if negative && decimal != [0] {
         f.write_str("-")?;
      }
      for digit in decimal.iter().rev() {
         write!(f, "{digit}")?;
      }
      Ok(())
   }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Radix {
   Binary,
   Octal,
   Decimal,
   Hexadecimal,
}

impl Radix {
   #[must_use]
   #[inline]
   pub const fn accepts(self, digit: char) -> bool {
      match self {
         Self::Binary => matches!(digit, '0' | '1'),
         Self::Octal => matches!(digit, '0'..='7'),
         Self::Decimal => digit.is_ascii_digit(),
         Self::Hexadecimal => digit.is_ascii_hexdigit(),
      }
   }
}

impl From<Radix> for u32 {
   #[inline]
   fn from(radix: Radix) -> Self {
      match radix {
         Radix::Binary => 2,
         Radix::Octal => 8,
         Radix::Decimal => 10,
         Radix::Hexadecimal => 16,
      }
   }
}
