use std::iter::from_fn;

use proc_macro2::{
   Delimiter,
   Group,
   Ident,
   TokenStream as Tokens,
   TokenTree,
};
use venial::{
   Attribute,
   AttributeValue,
   Error,
   TypeExpr,
};

use crate::syntax::{
   decode_string,
   expression_end,
};

/// Node location or decoding strategy used to populate a struct field.
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Source {
   /// Reads the next positional argument from the node.
   Argument,
   /// Collects all remaining positional arguments into a collection.
   Arguments,
   /// Reads a property matching the field name or explicit attribute name.
   Property,
   /// Collects all remaining properties into a map structure.
   Properties,
   /// Decodes a single nested child node matching the field or specified name.
   Child,
   /// Collects multiple nested child nodes into a collection.
   Children,
   /// Delegates decoding of the current node to the field type directly.
   Flatten,
   /// Parses the tag identifier of the node itself into the field.
   NodeName,
}

/// Strategy for extracting inner values from a child node instead of decoding a
/// nested struct.
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Unwrap {
   /// Extracts the single positional argument from the child node.
   Argument,
   /// Extracts all positional arguments from the child node into a collection.
   Arguments,
   /// Extracts all properties from the child node into a key-value collection.
   Properties,
}

/// Fallback value used when an optional or absent field is not present in the
/// node.
pub enum DefaultValue {
   /// Uses the standard `Default` trait implementation for the field type.
   Implicit,
   /// Evaluates a custom expression provided by the user in the attribute.
   Expression(Tokens),
}

/// Validated configuration parsed from `#[knead(...)]` attributes on a field.
pub struct Options {
   /// Location within the KDL node where the field value originates.
   pub source:    Source,
   /// Explicit KDL identifier override when different from the Rust field name.
   pub name:      Option<String>,
   /// Fallback construction rule applied when the field is omitted from the
   /// input.
   pub default:   Option<DefaultValue>,
   /// Shorthand extraction mode applied to child nodes without full struct
   /// decoding.
   pub unwrap:    Option<Unwrap>,
   /// Whether to decode the value by parsing a string representation via
   /// `FromStr`.
   pub str_value: bool,
}

/// Intermediate attribute collector used to aggregate settings before
/// validation.
struct Accum {
   /// Staged node location setting parsed from field attributes.
   source:   Option<Source>,
   /// Staged explicit name override parsed from field attributes.
   name:     Option<String>,
   /// Staged fallback value rule parsed from field attributes.
   default:  Option<DefaultValue>,
   /// Staged child unwrapping mode parsed from field attributes.
   unwrap:   Option<Unwrap>,
   /// Staged flag indicating whether `str` parsing was requested.
   str_flag: bool,
}

/// Extracts and validates decoding options from the `knead` attributes on a
/// struct field.
#[expect(
   clippy::too_many_lines,
   reason = "one validation pass reads better than split halves"
)]
pub fn field_options(attributes: &[Attribute], ty: &TypeExpr) -> Result<Options, Error> {
   let mut accum = Accum {
      source:   None,
      name:     None,
      default:  None,
      unwrap:   None,
      str_flag: false,
   };
   for attr_ref in attributes {
      let Some(segment) = attr_ref.get_single_path_segment() else {
         continue;
      };
      if segment != "knead" {
         continue;
      }
      let AttributeValue::Group(ref group, ref inner) = attr_ref.value else {
         return Err(Error::new_at_tokens(attr_ref, "expected parentheses here"));
      };
      if group.delimiter != Delimiter::Parenthesis {
         return Err(Error::new_at_tokens(attr_ref, "expected parentheses here"));
      }
      for item in items(inner) {
         parse_top(item?, &mut accum)?;
      }
   }
   let Some(found_source) = accum.source else {
      return Err(Error::new_at_tokens(ty, "field needs a knead attribute"));
   };
   if found_source == Source::Flatten {
      if accum.name.is_some() || accum.default.is_some() || accum.unwrap.is_some() || accum.str_flag
      {
         return Err(Error::new_at_tokens(
            ty,
            "flatten cannot be combined with other options",
         ));
      }
   } else if found_source == Source::NodeName {
      if accum.default.is_some() {
         return Err(Error::new_at_tokens(
            ty,
            "default cannot be used with node_name or flatten",
         ));
      }
      if accum.name.is_some() {
         return Err(Error::new_at_tokens(
            ty,
            "name fits only property child and children fields",
         ));
      }
      if accum.unwrap.is_some() {
         return Err(Error::new_at_tokens(
            ty,
            "unwrap fits only child and children fields",
         ));
      }
      if accum.str_flag {
         return Err(Error::new_at_tokens(
            ty,
            "str needs argument property or unwrap argument",
         ));
      }
   } else {
      if accum.name.is_some()
         && found_source != Source::Property
         && found_source != Source::Child
         && found_source != Source::Children
      {
         return Err(Error::new_at_tokens(
            ty,
            "name fits only property child and children fields",
         ));
      }
      if accum.unwrap.is_some() && found_source != Source::Child && found_source != Source::Children
      {
         return Err(Error::new_at_tokens(
            ty,
            "unwrap fits only child and children fields",
         ));
      }
      if accum.str_flag && accum.unwrap == Some(Unwrap::Properties) {
         return Err(Error::new_at_tokens(ty, "str fits only unwrap argument"));
      }
      if accum.str_flag && accum.unwrap == Some(Unwrap::Arguments) {
         return Err(Error::new_at_tokens(ty, "str fits only unwrap argument"));
      }
      if accum.str_flag
         && (found_source == Source::Child || found_source == Source::Children)
         && accum.unwrap != Some(Unwrap::Argument)
      {
         return Err(Error::new_at_tokens(
            ty,
            "str needs argument property or unwrap argument",
         ));
      }
   }
   Ok(Options {
      source:    found_source,
      name:      accum.name,
      default:   accum.default,
      unwrap:    accum.unwrap,
      str_value: accum.str_flag,
   })
}

/// Parses a single comma-separated attribute entry and dispatches to the
/// appropriate parser.
fn parse_top(item: &[TokenTree], accum: &mut Accum) -> Result<(), Error> {
   let first = &item[0];
   let TokenTree::Ident(ref ident) = *first else {
      return Err(Error::new_at_span(
         first.span(),
         "expected a simple name here",
      ));
   };
   let rest = &item[1..];
   if rest.is_empty() {
      return parse_bare(ident, accum);
   }
   if rest.len() == 1
      && let TokenTree::Group(ref group) = rest[0]
   {
      if group.delimiter() == Delimiter::Parenthesis {
         return parse_list(ident, group, accum);
      }
      return Err(Error::new_at_span(
         rest[0].span(),
         "expected a simple name here",
      ));
   }
   if let Some(second) = rest.first()
      && is_equals(second)
   {
      return parse_assign(ident, rest, accum);
   }
   Err(Error::new_at_span(
      rest[0].span(),
      "expected a simple name here",
   ))
}

/// Records a standalone identifier attribute without arguments into the
/// accumulator.
fn parse_bare(ident: &Ident, accum: &mut Accum) -> Result<(), Error> {
   let text = ident.to_string();
   let found = match text.as_str() {
      "argument" => Some(Source::Argument),
      "arguments" => Some(Source::Arguments),
      "property" => Some(Source::Property),
      "properties" => Some(Source::Properties),
      "child" => Some(Source::Child),
      "children" => Some(Source::Children),
      "flatten" => Some(Source::Flatten),
      "node_name" => Some(Source::NodeName),
      _ => None,
   };
   if let Some(next) = found {
      if accum.source.is_some() {
         return Err(Error::new_at_span(
            ident.span(),
            "conflicting knead attributes on one field",
         ));
      }
      accum.source = Some(next);
      return Ok(());
   }
   if text == "str" {
      if accum.str_flag {
         return Err(Error::new_at_span(
            ident.span(),
            "duplicate str on one field",
         ));
      }
      accum.str_flag = true;
      return Ok(());
   }
   if text == "default" {
      if accum.default.is_some() {
         return Err(Error::new_at_span(
            ident.span(),
            "duplicate default on one field",
         ));
      }
      accum.default = Some(DefaultValue::Implicit);
      return Ok(());
   }
   Err(Error::new_at_span(
      ident.span(),
      format!("unknown knead attribute `{text}`"),
   ))
}

/// Dispatches parenthesized attribute lists like `child(...)` or `unwrap(...)`
/// to inner parsers.
fn parse_list(ident: &Ident, group: &Group, accum: &mut Accum) -> Result<(), Error> {
   let text = ident.to_string();
   let inner = group.stream().into_iter().collect::<Vec<_>>();
   if text == "child" || text == "property" || text == "children" {
      parse_child_inner(ident, &inner, accum)
   } else if text == "unwrap" {
      parse_unwrap_inner(ident, &inner, accum)
   } else {
      Err(Error::new_at_span(
         ident.span(),
         format!("unknown knead attribute `{text}`"),
      ))
   }
}

/// Parses entries nested inside a `child`, `property`, or `children` attribute
/// list.
fn parse_child_inner(ident: &Ident, inner: &[TokenTree], accum: &mut Accum) -> Result<(), Error> {
   let text = ident.to_string();
   let next = if text == "child" {
      Source::Child
   } else if text == "property" {
      Source::Property
   } else {
      Source::Children
   };
   if accum.source.is_some() {
      return Err(Error::new_at_span(
         ident.span(),
         "conflicting knead attributes on one field",
      ));
   }
   for nested in items(inner) {
      parse_child_entry(nested?, accum)?;
   }
   accum.source = Some(next);
   Ok(())
}

/// Parses a `name = "..."` assignment entry inside child or property attribute
/// lists.
fn parse_child_entry(nested: &[TokenTree], accum: &mut Accum) -> Result<(), Error> {
   let first = &nested[0];
   let fallback = first.span();
   let TokenTree::Ident(ref name_ident) = *first else {
      return Err(Error::new_at_span(
         fallback,
         "expected name inside child property or children",
      ));
   };
   if name_ident != "name" {
      return Err(Error::new_at_span(
         name_ident.span(),
         "expected name inside child property or children",
      ));
   }
   let Some(second) = nested.get(1) else {
      return Err(Error::new_at_span(
         name_ident.span(),
         "expected name inside child property or children",
      ));
   };
   if !is_equals(second) {
      return Err(Error::new_at_span(
         second.span(),
         "expected name inside child property or children",
      ));
   }
   let value_tokens = &nested[2..];
   if value_tokens.len() != 1 {
      let complaint = value_tokens
         .first()
         .map_or_else(|| name_ident.span(), TokenTree::span);
      return Err(Error::new_at_span(complaint, "name needs a string literal"));
   }
   let TokenTree::Literal(ref literal) = value_tokens[0] else {
      return Err(Error::new_at_span(
         value_tokens[0].span(),
         "name needs a string literal",
      ));
   };
   let Some(decoded) = decode_string(literal) else {
      return Err(Error::new_at_span(
         literal.span(),
         "name needs a string literal",
      ));
   };
   if accum.name.is_some() {
      return Err(Error::new_at_span(
         name_ident.span(),
         "duplicate name on one field",
      ));
   }
   accum.name = Some(decoded);
   Ok(())
}

/// Parses the target kind and optional `str` flag specified inside an
/// `unwrap(...)` attribute.
fn parse_unwrap_inner(ident: &Ident, inner: &[TokenTree], accum: &mut Accum) -> Result<(), Error> {
   let parts = items(inner).collect::<Result<Vec<_>, _>>()?;
   if parts.is_empty() {
      return Err(Error::new_at_span(
         ident.span(),
         "expected unwrap argument unwrap arguments or unwrap properties",
      ));
   }
   if parts.len() > 2 {
      return Err(Error::new_at_span(
         ident.span(),
         "too many items inside unwrap",
      ));
   }
   let first_span = parts[0][0].span();
   let Some(first_text) = single_ident_text(parts[0]) else {
      return Err(Error::new_at_span(
         first_span,
         "expected unwrap argument unwrap arguments or unwrap properties",
      ));
   };
   let chosen = match first_text.as_str() {
      "argument" => Unwrap::Argument,
      "arguments" => Unwrap::Arguments,
      "properties" => Unwrap::Properties,
      _ => {
         return Err(Error::new_at_span(
            first_span,
            "expected unwrap argument unwrap arguments or unwrap properties",
         ));
      },
   };
   let with_str = if parts.len() == 2 {
      let second_span = parts[1][0].span();
      let Some(second_text) = single_ident_text(parts[1]) else {
         return Err(Error::new_at_span(
            second_span,
            "expected str after unwrap argument",
         ));
      };
      if second_text != "str" {
         return Err(Error::new_at_span(
            second_span,
            "expected str after unwrap argument",
         ));
      }
      true
   } else {
      false
   };
   if with_str && chosen != Unwrap::Argument {
      return Err(Error::new_at_span(
         ident.span(),
         "str fits only unwrap argument",
      ));
   }
   if accum.unwrap.is_some() {
      return Err(Error::new_at_span(
         ident.span(),
         "duplicate unwrap on one field",
      ));
   }
   accum.unwrap = Some(chosen);
   if with_str {
      if accum.str_flag {
         return Err(Error::new_at_span(
            ident.span(),
            "duplicate str on one field",
         ));
      }
      accum.str_flag = true;
   }
   Ok(())
}

/// Parses an assignment attribute such as `default = <expr>` into the
/// accumulator.
fn parse_assign(ident: &Ident, rest: &[TokenTree], accum: &mut Accum) -> Result<(), Error> {
   let text = ident.to_string();
   if text == "default" {
      if accum.default.is_some() {
         return Err(Error::new_at_span(
            ident.span(),
            "duplicate default on one field",
         ));
      }
      let expr_tokens = &rest[1..];
      if expr_tokens.is_empty() {
         return Err(Error::new_at_span(
            ident.span(),
            "default needs an expression here",
         ));
      }
      let stream = expr_tokens.iter().cloned().collect::<Tokens>();
      accum.default = Some(DefaultValue::Expression(stream));
      return Ok(());
   }
   if text == "name" {
      return Err(Error::new_at_span(
         ident.span(),
         "name fits only property child and children fields",
      ));
   }
   Err(Error::new_at_span(ident.span(), "expected default here"))
}

/// Iterates over comma-separated token sequences while respecting nested
/// expressions.
fn items(mut remaining: &[TokenTree]) -> impl Iterator<Item = Result<&[TokenTree], Error>> {
   from_fn(move || {
      if remaining.is_empty() {
         return None;
      }
      let length = expression_end(remaining);
      let part = &remaining[..length];
      let first_span = remaining[0].span();
      remaining = remaining.get(length + 1..).unwrap_or_default();
      Some(if part.is_empty() {
         Err(Error::new_at_span(
            first_span,
            "expected a simple name here",
         ))
      } else {
         Ok(part)
      })
   })
}

/// Returns whether a token is an ASCII equals sign punctuation mark.
fn is_equals(token: &TokenTree) -> bool {
   matches!(token, TokenTree::Punct(punct) if punct.as_char() == '=')
}

/// Extracts the identifier name if the token slice contains exactly one
/// identifier token.
fn single_ident_text(tokens: &[TokenTree]) -> Option<String> {
   if tokens.len() != 1 {
      return None;
   }
   if let TokenTree::Ident(ref ident) = tokens[0] {
      Some(ident.to_string())
   } else {
      None
   }
}
