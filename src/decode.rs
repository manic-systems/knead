use std::{
   borrow::Cow,
   path::PathBuf,
};

use crate::{
   ast::{
      Literal,
      Node,
      Value,
   },
   dialect::Dialect,
   errors::{
      Error,
      ErrorKind,
   },
   span::{
      Span,
      Spanned,
   },
};

pub trait Decode: Sized {
   const DIALECT: Dialect = Dialect::V2;

   /// # Errors
   ///
   /// Fails when the node does not fit `Self`.
   fn decode(decoder: &mut Decoder<'_>) -> Result<Self, Error>;

   /// # Errors
   ///
   /// Fails when the node does not fit `Self` or leaves an entry unused.
   #[inline]
   fn decode_node(node: &Node) -> Result<Self, Error> {
      let mut decoder = Decoder::new(node);
      let value = Self::decode(&mut decoder)?;
      decoder.finish()?;
      Ok(value)
   }
}

pub trait DecodeScalar: Sized {
   /// # Errors
   ///
   /// Fails when the value carries a type annotation `Self` does not accept.
   #[inline]
   fn type_check(value: &Value<'_>) -> Result<(), Error> {
      value.reject_type()
   }

   /// # Errors
   ///
   /// Fails when the value does not fit `Self`.
   fn decode(value: &Value<'_>) -> Result<Self, Error>;
}

pub struct Decoder<'src> {
   /// Node whose arguments, properties, and children are being handed out.
   node:          &'src Node<'src>,
   /// Replacement name reported by `name`, set when a caller renames the node.
   name_override: Option<Spanned<Cow<'src, str>>>,
   /// Index of the next argument that has not been handed out yet.
   arg_next:      usize,
   /// One flag per property recording which ones have been claimed.
   props_used:    Used,
   /// One flag per child node recording which ones have been claimed.
   children_used: Used,
}

impl<'src> Decoder<'src> {
   #[must_use]
   #[inline]
   pub fn new(node: &'src Node<'src>) -> Self {
      Self {
         node,
         name_override: None,
         arg_next: 0,
         props_used: Used::new(node.properties.len()),
         children_used: Used::new(node.child_nodes().len()),
      }
   }

   #[must_use]
   #[inline]
   pub const fn node(&self) -> &'src Node<'src> {
      self.node
   }

   #[must_use]
   #[inline]
   pub fn name(&self) -> &Spanned<Cow<'src, str>> {
      self.name_override.as_ref().unwrap_or(&self.node.name)
   }

   #[inline]
   pub fn set_name<Text>(&mut self, name: Spanned<Text>)
   where
      Text: Into<Cow<'src, str>>,
   {
      self.name_override = Some(Spanned::new(name.value.into(), name.span));
   }

   #[must_use]
   #[inline]
   pub const fn span(&self) -> Span {
      self.node.span
   }

   #[must_use]
   #[inline]
   pub const fn type_name(&self) -> Option<&'src Spanned<Cow<'src, str>>> {
      self.node.type_name.as_ref()
   }

   /// # Errors
   ///
   /// Fails when the node carries a type annotation.
   #[inline]
   pub fn reject_type(&self) -> Result<(), Error> {
      self.node.reject_type()
   }

   #[inline]
   pub fn argument(&mut self) -> Option<&'src Value<'src>> {
      let value = self.node.arguments.get(self.arg_next)?;
      self.arg_next += 1;
      Some(value)
   }

   #[inline]
   pub fn arguments(&mut self) -> &'src [Value<'src>] {
      let rest = &self.node.arguments[self.arg_next..];
      self.arg_next = self.node.arguments.len();
      rest
   }

   #[inline]
   pub fn property(&mut self, name: &str) -> Option<&'src Value<'src>> {
      let mut found = None;
      for (index, prop) in self.node.properties.iter().enumerate() {
         if self.props_used.get(index) || prop.name.value != name {
            continue;
         }
         self.props_used.set(index);
         found = Some(&prop.value);
      }
      found
   }

   #[inline]
   pub fn properties(&mut self) -> impl Iterator<Item = (&'src str, &'src Value<'src>)> {
      let used = &mut self.props_used;
      self
         .node
         .properties
         .iter()
         .enumerate()
         .filter(move |&(index, _)| used.claim(index))
         .map(|(_, prop)| (&*prop.name.value, &prop.value))
   }

   /// # Errors
   ///
   /// Fails when more than one unused child is named `name`.
   #[inline]
   pub fn child(&mut self, name: &str) -> Result<Option<&'src Node<'src>>, Error> {
      let nodes = self.node.child_nodes();
      let mut found = None;
      for (index, child) in nodes.iter().enumerate() {
         if self.children_used.get(index) || child.name.value != name {
            continue;
         }
         if found.is_some() {
            return Err(Error::new(
               ErrorKind::Duplicate,
               child.name.span,
               format!("duplicate child {name:?}"),
            ));
         }
         found = Some(index);
      }
      Ok(found.map(|index| {
         self.children_used.set(index);
         &nodes[index]
      }))
   }

   #[inline]
   pub fn children(&mut self, name: Option<&str>) -> impl Iterator<Item = &'src Node<'src>> {
      let used = &mut self.children_used;
      self
         .node
         .child_nodes()
         .iter()
         .enumerate()
         .filter(move |&(index, child)| {
            name.is_none_or(|want| child.name.value == want) && used.claim(index)
         })
         .map(|(_, child)| child)
   }

   /// # Errors
   ///
   /// Fails when the node is annotated, has no argument, or has anything beyond
   /// that one argument.
   #[inline]
   pub fn unwrap_argument(mut self) -> Result<&'src Value<'src>, Error> {
      self.reject_type()?;
      let value = self.argument().ok_or_else(|| {
         Error::new(
            ErrorKind::Missing,
            self.node.name.span,
            "one argument is required",
         )
      })?;
      self.finish()?;
      Ok(value)
   }

   /// # Errors
   ///
   /// Fails when the node is annotated or has properties or children.
   #[inline]
   pub fn unwrap_arguments(mut self) -> Result<&'src [Value<'src>], Error> {
      self.reject_type()?;
      let values = self.arguments();
      self.finish()?;
      Ok(values)
   }

   /// # Errors
   ///
   /// Fails when an argument, property, or child was left unused.
   #[inline]
   pub fn finish(&self) -> Result<(), Error> {
      if let Some(value) = self.node.arguments.get(self.arg_next) {
         return Err(Error::new(
            ErrorKind::Unexpected,
            value.span,
            "unexpected argument".to_owned(),
         ));
      }
      for (index, prop) in self.node.properties.iter().enumerate() {
         if self.props_used.get(index) {
            continue;
         }
         return Err(Error::new(
            ErrorKind::Unexpected,
            prop.name.span,
            format!("unexpected property {:?}", prop.name.value),
         ));
      }
      for (index, child) in self.node.child_nodes().iter().enumerate() {
         if self.children_used.get(index) {
            continue;
         }
         return Err(Error::new(
            ErrorKind::Unexpected,
            child.name.span,
            format!("unexpected node {:?}", child.name.value),
         ));
      }
      Ok(())
   }
}

/// Bitset sized once in `new`, keeping the first 64 flags inline and the rest
/// in a spill vector.
struct Used {
   /// Flags for indices below 64.
   inline: u64,
   /// Flags for indices from 64 upward, in words of 64.
   spill:  Vec<u64>,
}

impl Used {
   /// Creates a cleared bitset able to hold `len` flags.
   fn new(len: usize) -> Self {
      Self {
         inline: 0,
         spill:  vec![0; len.saturating_sub(64).div_ceil(64)],
      }
   }

   /// Locates the word holding `index` and the mask for its bit within that
   /// word.
   fn word(&mut self, index: usize) -> (&mut u64, u64) {
      let mask = 1 << (index % 64);
      match index.checked_sub(64) {
         None => (&mut self.inline, mask),
         Some(spilled) => (&mut self.spill[spilled / 64], mask),
      }
   }

   /// Reports whether the flag at `index` is set.
   fn get(&self, index: usize) -> bool {
      let mask = 1 << (index % 64);
      let word = index
         .checked_sub(64)
         .map_or(self.inline, |spilled| self.spill[spilled / 64]);
      word & mask != 0
   }

   /// Sets the flag at `index` unconditionally.
   fn set(&mut self, index: usize) {
      let (word, mask) = self.word(index);
      *word |= mask;
   }

   /// Sets the flag at `index` and reports whether it was previously clear.
   fn claim(&mut self, index: usize) -> bool {
      let (word, mask) = self.word(index);
      let fresh = *word & mask == 0;
      *word |= mask;
      fresh
   }
}

/// Implements `DecodeScalar` for an integer type by parsing the literal digits
/// in their radix.
macro_rules! impl_int {
   ($type:ty, $name:expr) => {
      impl DecodeScalar for $type {
         #[inline]
         fn type_check(value: &Value<'_>) -> Result<(), Error> {
            value.expect_type(&[$name])
         }

         #[inline]
         fn decode(value: &Value<'_>) -> Result<Self, Error> {
            Self::type_check(value)?;
            let Literal::Integer(ref number) = value.literal else {
               return Err(value.type_mismatch("integer"));
            };
            Self::from_str_radix(&number.digits, u32::from(number.radix))
               .map_err(|error| Error::conversion(value.span, error))
         }
      }
   };
}

impl_int!(i8, "i8");
impl_int!(u8, "u8");
impl_int!(i16, "i16");
impl_int!(u16, "u16");
impl_int!(i32, "i32");
impl_int!(u32, "u32");
impl_int!(i64, "i64");
impl_int!(u64, "u64");
impl_int!(i128, "i128");
impl_int!(u128, "u128");
impl_int!(isize, "isize");
impl_int!(usize, "usize");

/// Implements `DecodeScalar` for a float type by parsing the decimal literal
/// text.
macro_rules! impl_float {
   ($type:ty, $name:expr) => {
      impl DecodeScalar for $type {
         #[inline]
         fn type_check(value: &Value<'_>) -> Result<(), Error> {
            value.expect_type(&[$name])
         }

         #[inline]
         fn decode(value: &Value<'_>) -> Result<Self, Error> {
            Self::type_check(value)?;
            let Literal::Decimal(ref text) = value.literal else {
               return Err(value.type_mismatch("decimal"));
            };
            match &**text {
               "#inf" => return Ok(Self::INFINITY),
               "#-inf" => return Ok(Self::NEG_INFINITY),
               "#nan" => return Ok(Self::NAN),
               _ => {},
            }
            let number = text
               .parse::<Self>()
               .map_err(|error| Error::conversion(value.span, error))?;
            if number.is_infinite() {
               return Err(Error::new(
                  ErrorKind::Conversion,
                  value.span,
                  format!("decimal {:?} does not fit in {}", text, $name),
               ));
            }
            Ok(number)
         }
      }
   };
}

impl_float!(f32, "f32");
impl_float!(f64, "f64");

impl DecodeScalar for String {
   #[inline]
   fn type_check(value: &Value<'_>) -> Result<(), Error> {
      value.expect_type(&["str", "string"])
   }

   #[inline]
   fn decode(value: &Value<'_>) -> Result<Self, Error> {
      Self::type_check(value)?;
      value.expect_string().map(str::to_owned)
   }
}

impl DecodeScalar for bool {
   #[inline]
   fn decode(value: &Value<'_>) -> Result<Self, Error> {
      Self::type_check(value)?;
      let Literal::Bool(flag) = value.literal else {
         return Err(value.type_mismatch("boolean"));
      };
      Ok(flag)
   }
}

impl DecodeScalar for PathBuf {
   #[inline]
   fn type_check(value: &Value<'_>) -> Result<(), Error> {
      value.expect_type(&["str", "string"])
   }

   #[inline]
   fn decode(value: &Value<'_>) -> Result<Self, Error> {
      Self::type_check(value)?;
      value.expect_string().map(Self::from)
   }
}

impl<Inner: DecodeScalar> DecodeScalar for Option<Inner> {
   #[inline]
   fn type_check(value: &Value<'_>) -> Result<(), Error> {
      Inner::type_check(value)
   }

   #[inline]
   fn decode(value: &Value<'_>) -> Result<Self, Error> {
      if value.literal == Literal::Null {
         Inner::type_check(value)?;
         return Ok(None);
      }
      Inner::decode(value).map(Some)
   }
}

impl<Inner: Decode> Decode for Box<Inner> {
   #[inline]
   fn decode(decoder: &mut Decoder<'_>) -> Result<Self, Error> {
      Ok(Self::new(Inner::decode(decoder)?))
   }
}

impl<Inner: DecodeScalar> DecodeScalar for Box<Inner> {
   #[inline]
   fn type_check(value: &Value<'_>) -> Result<(), Error> {
      Inner::type_check(value)
   }

   #[inline]
   fn decode(value: &Value<'_>) -> Result<Self, Error> {
      Ok(Self::new(Inner::decode(value)?))
   }
}
