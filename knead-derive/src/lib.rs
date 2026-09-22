/// Parser and validator for `#[knead(...)]` derive attributes.
mod attributes;
/// Token stream manipulation and expression boundary analysis for macro inputs.
mod syntax;

use std::{
   collections::BTreeSet,
   result::Result as StdResult,
};

use proc_macro::TokenStream;
use proc_macro2::{
   Delimiter,
   Ident,
   Span,
   TokenStream as Tokens,
   TokenTree,
};
use quote::{
   ToTokens,
   format_ident,
   quote,
};
use venial::{
   Attribute,
   AttributeValue,
   Error,
   Fields,
   GenericArg,
   Item,
   TypeExpr,
};

use crate::{
   attributes::{
      DefaultValue,
      Options,
      Source,
      Unwrap,
      field_options,
   },
   syntax::parse_derive,
};

/// Convenience alias for results carrying venial parsing errors.
type Result<Output> = StdResult<Output, Error>;

#[inline]
#[proc_macro_derive(Decode, attributes(knead))]
pub fn derive_decode(input: TokenStream) -> TokenStream {
   parse_derive(input.into())
      .and_then(|item| decode_impl(&item))
      .unwrap_or_else(|error| error.to_compile_error())
      .into()
}

#[inline]
#[proc_macro_derive(DecodeScalar, attributes(knead))]
pub fn derive_scalar(input: TokenStream) -> TokenStream {
   parse_derive(input.into())
      .and_then(|item| scalar_impl(&item))
      .unwrap_or_else(|error| error.to_compile_error())
      .into()
}

/// Ensures that no misplaced `knead` attributes appear on containers or enum
/// variants.
fn reject_attributes(attributes: &[Attribute]) -> Result<()> {
   for attribute in attributes {
      if attribute
         .get_single_path_segment()
         .is_some_and(|name| name == "knead")
      {
         return Err(Error::new_at_tokens(
            attribute,
            "knead attributes belong on fields",
         ));
      }
   }
   Ok(())
}

/// Parses the container-level dialect attribute and returns code evaluating to
/// the chosen dialect.
fn container_dialect(attributes: &[Attribute]) -> Result<Tokens> {
   let mut dialect = quote!(::knead::dialect::Dialect::V2);
   for attribute in attributes {
      if attribute
         .get_single_path_segment()
         .is_none_or(|name| name != "knead")
      {
         continue;
      }
      let AttributeValue::Group(ref group, ref inner) = attribute.value else {
         return Err(Error::new_at_tokens(attribute, "expected parentheses here"));
      };
      if group.delimiter != Delimiter::Parenthesis {
         return Err(Error::new_at_tokens(attribute, "expected parentheses here"));
      }
      let text = inner.iter().map(ToString::to_string).collect::<String>();
      dialect = match text.as_str() {
         "dialect=\"v1\"" => quote!(::knead::dialect::Dialect::V1),
         "dialect=\"v2\"" => quote!(::knead::dialect::Dialect::V2),
         _ => {
            return Err(Error::new_at_tokens(
               attribute,
               "the only container attribute is dialect = \"v1\" or \"v2\"",
            ));
         },
      };
   }
   Ok(dialect)
}

/// Generates the `Decode` trait implementation for a struct or enum item.
fn decode_impl(input: &Item) -> Result<Tokens> {
   let dialect = container_dialect(input.attributes())?;
   let mut bounds = Vec::new();
   let (ident, generics, existing_where, body) = if let Item::Struct(ref data) = *input {
      (
         &data.name,
         &data.generic_params,
         &data.where_clause,
         decode_fields(&data.fields, quote!(Self), &mut bounds)?,
      )
   } else if let Item::Enum(ref data) = *input {
      let mut names = BTreeSet::new();
      let mut arms = Vec::new();
      for variant in data.variants.items() {
         reject_attributes(&variant.attributes)?;
         let name = kebab(&variant.name.to_string());
         if !names.insert(name.clone()) {
            return Err(Error::new_at_tokens(
               variant,
               "duplicate decoded variant name",
            ));
         }
         let variant_name = &variant.name;
         let branch = decode_fields(&variant.fields, quote!(Self::#variant_name), &mut bounds)?;
         arms.push(quote!(#name => { #branch }));
      }
      let expected = format!(
         "expected one of {}",
         names.into_iter().collect::<Vec<_>>().join(", ")
      );
      let body = quote!(match &*decoder.name().value {
         #(#arms,)*
         _ => Err(::knead::errors::Error::new(
            ::knead::errors::ErrorKind::Conversion,
            decoder.name().span,
            #expected,
         )),
      });
      (&data.name, &data.generic_params, &data.where_clause, body)
   } else {
      return Err(Error::new_at_tokens(
         input,
         "Decode requires a struct or enum",
      ));
   };
   let parameter_names = generics
      .iter()
      .flat_map(|params| params.params.items())
      .filter(|param| param.is_ty())
      .map(|param| param.name.to_string())
      .collect();
   let mut predicates = existing_where
      .iter()
      .flat_map(|clause| clause.items.items())
      .map(ToTokens::to_token_stream)
      .collect::<Vec<_>>();
   for (target, bound) in bounds {
      if mentions(&target, &parameter_names)
         && !mentions(&target, &BTreeSet::from([ident.to_string()]))
      {
         predicates.push(bound);
      }
   }
   let type_generics = generics.as_ref().map(|params| params.as_inline_args());
   let where_clause = (!predicates.is_empty()).then(|| quote!(where #(#predicates),*));
   Ok(quote!(
      impl #generics ::knead::decode::Decode for #ident #type_generics #where_clause {
         const DIALECT: ::knead::dialect::Dialect = #dialect;

         fn decode(decoder: &mut ::knead::decode::Decoder<'_>)
            -> ::core::result::Result<Self, ::knead::errors::Error>
         {
            decoder.reject_type()?;
            #body
         }
      }
   ))
}

/// Generates the `DecodeScalar` trait implementation for an enum with unit
/// variants.
fn scalar_impl(input: &Item) -> Result<Tokens> {
   reject_attributes(input.attributes())?;
   let Item::Enum(ref data) = *input else {
      return Err(Error::new_at_tokens(
         input,
         "DecodeScalar requires an enum with unit variants",
      ));
   };
   let mut names = BTreeSet::new();
   let mut arms = Vec::new();
   for variant in data.variants.items() {
      reject_attributes(&variant.attributes)?;
      if !matches!(variant.fields, Fields::Unit) {
         return Err(Error::new_at_tokens(
            variant,
            "DecodeScalar requires unit variants",
         ));
      }
      let name = kebab(&variant.name.to_string());
      if !names.insert(name.clone()) {
         return Err(Error::new_at_tokens(
            variant,
            "duplicate decoded variant name",
         ));
      }
      let variant_name = &variant.name;
      arms.push(quote!(#name => Ok(Self::#variant_name)));
   }
   let expected = format!(
      "expected one of {}",
      names.into_iter().collect::<Vec<_>>().join(", ")
   );
   let ident = &data.name;
   let generics = &data.generic_params;
   let type_generics = data.get_inline_generic_args();
   let where_clause = &data.where_clause;
   Ok(quote!(
      impl #generics ::knead::decode::DecodeScalar for #ident #type_generics #where_clause {
         fn decode(value: &::knead::ast::Value<'_>)
            -> ::core::result::Result<Self, ::knead::errors::Error>
         {
            <Self as ::knead::decode::DecodeScalar>::type_check(value)?;
            match &value.literal {
               ::knead::ast::Literal::String(text) => match &**text {
                  #(#arms,)*
                  _ => Err(::knead::errors::Error::new(
                     ::knead::errors::ErrorKind::Conversion, value.span, #expected,
                  )),
               },
               _ => Err(::knead::errors::Error::new(
                  ::knead::errors::ErrorKind::Type, value.span, "expected a string",
               )),
            }
         }
      }
   ))
}

/// Collected metadata and generated identifiers for decoding a single struct
/// field.
struct FieldShape<'field> {
   /// Field identifier from named fields, or `None` for tuple fields.
   ident:   Option<&'field Ident>,
   /// Declared Rust type expression of the field.
   ty:      &'field TypeExpr,
   /// Parsed options controlling how the field is populated from the node.
   options: Options,
   /// Temporary local variable identifier bound during decoding.
   binding: Ident,
   /// KDL node name or argument index used when matching inputs.
   name:    String,
}

/// Collection of type and predicate pairs used to synthesize where-clause
/// bounds.
type Bounds = Vec<(Tokens, Tokens)>;

/// Emits field decoding logic ordered by priority so catch-all readers run
/// after specific ones.
#[expect(
   clippy::too_many_lines,
   reason = "one validation pass reads better than split halves"
)]
fn decode_fields(fields: &Fields, constructor: Tokens, bounds: &mut Bounds) -> Result<Tokens> {
   if let Fields::Tuple(ref unnamed) = *fields
      && unnamed.fields.len() == 1
      && let Some(field) = unnamed.fields.items().next()
      && !field.attributes.iter().any(|attribute| {
         attribute
            .get_single_path_segment()
            .is_some_and(|name| name == "knead")
      })
   {
      let ty = &field.ty;
      bounds.push((ty.to_token_stream(), quote!(#ty: ::knead::decode::Decode)));
      return Ok(quote!(Ok(#constructor(<#ty as ::knead::decode::Decode>::decode(decoder)?))));
   }
   let mut shapes = Vec::new();
   let mut keys = BTreeSet::new();
   let mut catchalls = BTreeSet::new();
   let entries = match *fields {
      Fields::Named(ref named) => {
         named
            .fields
            .items()
            .map(|field| (Some(&field.name), field.attributes.as_slice(), &field.ty))
            .collect::<Vec<_>>()
      },
      Fields::Tuple(ref unnamed) => {
         unnamed
            .fields
            .items()
            .map(|field| (None, field.attributes.as_slice(), &field.ty))
            .collect()
      },
      Fields::Unit => Vec::new(),
   };
   for (index, (ident, attributes, ty)) in entries.into_iter().enumerate() {
      let options = field_options(attributes, ty)?;
      let name = options.name.as_ref().map_or_else(
         || {
            ident.map_or_else(
               || format!("argument {index}"),
               |field_name| kebab(&field_name.to_string()),
            )
         },
         ToOwned::to_owned,
      );
      let namespace = match options.source {
         Source::Property => Some("property"),
         Source::Child => Some("child"),
         Source::Children if options.name.is_some() => Some("child"),
         Source::Argument
         | Source::Arguments
         | Source::Properties
         | Source::Children
         | Source::Flatten
         | Source::NodeName => None,
      };
      if let Some(namespace_kind) = namespace
         && !keys.insert((namespace_kind, name.clone()))
      {
         return Err(Error::new_at_tokens(
            ty,
            format!("duplicate {namespace_kind} name {name:?}"),
         ));
      }
      let catchall = match options.source {
         Source::Properties => Some("properties"),
         Source::Arguments => Some("arguments"),
         Source::NodeName => Some("node_name"),
         Source::Children if options.name.is_none() => Some("children"),
         Source::Argument
         | Source::Property
         | Source::Child
         | Source::Children
         | Source::Flatten => None,
      };
      if let Some(catchall_kind) = catchall
         && !catchalls.insert(catchall_kind)
      {
         return Err(Error::new_at_tokens(
            ty,
            format!("multiple {catchall_kind} fields"),
         ));
      }
      if options.source == Source::Argument && catchalls.contains("arguments") {
         return Err(Error::new_at_tokens(
            ty,
            "argument fields must precede arguments",
         ));
      }
      shapes.push(FieldShape {
         ident,
         ty,
         options,
         binding: format_ident!("knead_field_{index}", span = Span::mixed_site()),
         name,
      });
   }
   let priority = |shape: &FieldShape<'_>| -> u8 {
      match shape.options.source {
         Source::Properties => 2,
         Source::Children if shape.options.name.is_none() => 2,
         Source::Flatten => 1,
         Source::Argument
         | Source::Arguments
         | Source::Property
         | Source::Child
         | Source::Children
         | Source::NodeName => 0,
      }
   };
   let mut reads = Vec::new();
   for phase in 0..3 {
      for shape in shapes.iter().filter(|shape| priority(shape) == phase) {
         let binding = &shape.binding;
         let ty = shape.ty;
         let value = field_value(shape, bounds)?;
         reads.push(quote!(let #binding: #ty = #value;));
      }
   }
   let values = shapes.iter().map(|shape| &shape.binding);
   let value = match *fields {
      Fields::Named(_) => {
         let names = shapes.iter().map(|shape| shape.ident);
         quote!(#constructor { #(#names: #values),* })
      },
      Fields::Tuple(_) => quote!(#constructor(#(#values),*)),
      Fields::Unit => constructor,
   };
   Ok(quote!({ #(#reads)* Ok(#value) }))
}

/// Generates the decoding expression for a field according to its source and
/// unwrap options.
fn field_value(shape: &FieldShape<'_>, bounds: &mut Bounds) -> Result<Tokens> {
   let options = &shape.options;
   let ty = shape.ty;
   let name = &shape.name;
   let optional = option_inner(ty);
   match options.source {
      Source::Argument | Source::Property => {
         let lookup = if options.source == Source::Argument {
            quote!(decoder.argument())
         } else {
            quote!(decoder.property(#name))
         };
         let target = if options.str_value {
            optional.as_ref().unwrap_or(ty)
         } else {
            ty
         };
         let decoded = scalar(target, &quote!(value), options.str_value, bounds);
         let present = if optional.is_some() && options.str_value {
            quote!(Some(#decoded?))
         } else {
            quote!(#decoded?)
         };
         let absent = absent(shape, optional.is_some(), bounds);
         Ok(quote!(match #lookup { Some(value) => #present, None => #absent }))
      },
      Source::Arguments => {
         let item = collection_item(ty)?;
         let decoded = scalar(&item, &quote!(value), options.str_value, bounds);
         let values = quote!(decoder.arguments());
         Ok(collected(shape, &values, &quote!(value), &decoded))
      },
      Source::Properties => {
         let (key, item) = map_types(ty)?;
         let decoded = scalar(&item, &quote!(value), options.str_value, bounds);
         let values = quote!(decoder.properties());
         bounds.push((
            key.to_token_stream(),
            quote!(#key: ::core::convert::From<String>),
         ));
         Ok(collected(
            shape,
            &values,
            &quote!((key, value)),
            &quote!(Ok((key.to_owned().into(), #decoded?))),
         ))
      },
      Source::Child => {
         let target = optional.as_ref().unwrap_or(ty);
         let decoded = node_value(target, options, bounds)?;
         let present = if optional.is_some() {
            quote!(Some(#decoded?))
         } else {
            quote!(#decoded?)
         };
         let absent = absent(shape, optional.is_some(), bounds);
         Ok(quote!(match decoder.child(#name)? { Some(node) => #present, None => #absent }))
      },
      Source::Children => {
         let item = collection_item(ty)?;
         let decoded = node_value(&item, options, bounds)?;
         let filter = if options.name.is_some() {
            quote!(Some(#name))
         } else {
            quote!(None)
         };
         let values = quote!(decoder.children(#filter));
         Ok(collected(shape, &values, &quote!(node), &decoded))
      },
      Source::Flatten => {
         bounds.push((ty.to_token_stream(), quote!(#ty: ::knead::decode::Decode)));
         Ok(quote!(<#ty as ::knead::decode::Decode>::decode(decoder)?))
      },
      Source::NodeName => {
         str_bounds(ty, bounds);
         Ok(quote!(decoder.name().value.parse::<#ty>()
            .map_err(|error| ::knead::errors::Error::conversion(decoder.name().span, &error))?))
      },
   }
}

/// Generates the fallback value expression or missing error when an input node
/// entry is absent.
fn absent(shape: &FieldShape<'_>, optional: bool, bounds: &mut Bounds) -> Tokens {
   match shape.options.default {
      Some(DefaultValue::Expression(ref expression)) => quote!(#expression),
      Some(DefaultValue::Implicit) => {
         let ty = shape.ty;
         bounds.push((ty.to_token_stream(), quote!(#ty: ::core::default::Default)));
         quote!(::core::default::Default::default())
      },
      None if optional => quote!(None),
      None => {
         let kind = match shape.options.source {
            Source::Argument => "argument",
            Source::Property => "property",
            Source::Arguments
            | Source::Properties
            | Source::Child
            | Source::Children
            | Source::Flatten
            | Source::NodeName => "child node",
         };
         let message = format!("{kind} {:?} is required", shape.name);
         quote!(return Err(::knead::errors::Error::new(
            ::knead::errors::ErrorKind::Missing, decoder.name().span, #message,
         )))
      },
   }
}

/// Generates an expression decoding a scalar value via `FromStr` or
/// `DecodeScalar`.
fn scalar(ty: &TypeExpr, value: &Tokens, from_str: bool, bounds: &mut Bounds) -> Tokens {
   if from_str {
      str_bounds(ty, bounds);
      quote!((#value).parse_str::<#ty>())
   } else {
      bounds.push((
         ty.to_token_stream(),
         quote!(#ty: ::knead::decode::DecodeScalar),
      ));
      quote!(<#ty as ::knead::decode::DecodeScalar>::decode(#value))
   }
}

/// Appends `FromStr` and `Display` where-clause bounds required for
/// string-based parsing.
fn str_bounds(ty: &TypeExpr, bounds: &mut Bounds) {
   bounds.push((ty.to_token_stream(), quote!(#ty: ::core::str::FromStr)));
   bounds.push((
      ty.to_token_stream(),
      quote!(<#ty as ::core::str::FromStr>::Err: ::core::fmt::Display),
   ));
}

/// Generates a child node decoding expression, applying unwrap rules or full
/// node decoding.
fn node_value(ty: &TypeExpr, options: &Options, bounds: &mut Bounds) -> Result<Tokens> {
   match options.unwrap {
      None => {
         bounds.push((ty.to_token_stream(), quote!(#ty: ::knead::decode::Decode)));
         Ok(quote!(<#ty as ::knead::decode::Decode>::decode_node(node)))
      },
      Some(Unwrap::Argument) => {
         Ok(scalar(
            ty,
            &quote!(::knead::decode::Decoder::new(node).unwrap_argument()?),
            options.str_value,
            bounds,
         ))
      },
      Some(Unwrap::Arguments) => {
         let item = collection_item(ty)?;
         let decoded = scalar(&item, &quote!(value), options.str_value, bounds);
         Ok(
            quote!(::knead::decode::Decoder::new(node).unwrap_arguments()?.iter()
            .map(|value| -> ::core::result::Result<#item, ::knead::errors::Error> { #decoded })
            .collect::<::core::result::Result<#ty, _>>()),
         )
      },
      Some(Unwrap::Properties) => {
         let (key, item) = map_types(ty)?;
         let decoded = scalar(&item, &quote!(value), false, bounds);
         bounds.push((
            key.to_token_stream(),
            quote!(#key: ::core::convert::From<String>),
         ));
         Ok(quote!({
            let mut nested = ::knead::decode::Decoder::new(node);
            nested.reject_type()?;
            let properties = nested.properties().into_iter()
               .map(|(key, value)| -> ::core::result::Result<(#key, #item), ::knead::errors::Error> {
                  Ok((key.to_owned().into(), #decoded?))
               })
               .collect::<::core::result::Result<#ty, _>>()?;
            nested.finish()?;
            Ok::<#ty, ::knead::errors::Error>(properties)
         }))
      },
   }
}

/// Generates an iterator collection expression that maps and decodes multiple
/// input entries.
fn collected(
   shape: &FieldShape<'_>,
   values: &Tokens,
   binding: &Tokens,
   decoded: &Tokens,
) -> Tokens {
   let ty = shape.ty;
   let collect = quote!(entries
      .map(|#binding| -> ::core::result::Result<_, ::knead::errors::Error> { #decoded })
      .collect::<::core::result::Result<#ty, _>>()?);
   if let Some(DefaultValue::Expression(ref expression)) = shape.options.default {
      quote!({
         let mut entries = #values.into_iter().peekable();
         if entries.peek().is_none() { #expression } else { #collect }
      })
   } else {
      quote!({ let entries = #values.into_iter(); #collect })
   }
}

/// Extracts generic type arguments from the final segment of a type path
/// expression.
fn type_arguments(ty: &TypeExpr) -> Result<Vec<TypeExpr>> {
   if let Some(mut path) = ty.as_path()
      && let Some(segment) = path.segments.pop()
      && let Some(arguments) = segment.generic_args
   {
      return Ok(arguments
         .args
         .inner
         .into_iter()
         .filter_map(|(argument, _)| {
            match argument {
               GenericArg::TypeOrConst { expr } => Some(expr),
               GenericArg::Lifetime { .. } | GenericArg::Binding { .. } => None,
            }
         })
         .collect());
   }
   Err(Error::new_at_tokens(
      ty,
      "expected a collection type with type arguments",
   ))
}

/// Extracts the first generic argument representing the element type of a
/// collection.
fn collection_item(ty: &TypeExpr) -> Result<TypeExpr> {
   type_arguments(ty)?
      .into_iter()
      .next()
      .ok_or_else(|| Error::new_at_tokens(ty, "expected a collection item type"))
}

/// Extracts the key and value type arguments from a map type expression.
fn map_types(ty: &TypeExpr) -> Result<(TypeExpr, TypeExpr)> {
   let mut arguments = type_arguments(ty)?.into_iter();
   match (arguments.next(), arguments.next()) {
      (Some(key), Some(value)) => Ok((key, value)),
      _ => {
         Err(Error::new_at_tokens(
            ty,
            "expected a map with key and value types",
         ))
      },
   }
}

/// Returns the inner type expression if the given type is an `Option`, or
/// `None` otherwise.
fn option_inner(ty: &TypeExpr) -> Option<TypeExpr> {
   let path = ty.as_path()?;
   let segment = path.segments.last()?;
   if segment.ident != "Option" {
      return None;
   }
   type_arguments(ty).ok()?.into_iter().next()
}

/// Checks whether any identifier matching the set of names appears within the
/// token stream.
fn mentions(tokens: &Tokens, names: &BTreeSet<String>) -> bool {
   tokens.clone().into_iter().any(|token| {
      match token {
         TokenTree::Ident(ident) => names.contains(&ident.to_string()),
         TokenTree::Group(group) => mentions(&group.stream(), names),
         TokenTree::Punct(_) | TokenTree::Literal(_) => false,
      }
   })
}

/// Converts a Rust identifier from camel or snake case into kebab case for KDL
/// naming.
fn kebab(name: &str) -> String {
   let original = name.strip_prefix("r#").unwrap_or(name);
   let letters = original.chars().collect::<Vec<_>>();
   let mut result = String::new();
   for (index, character) in letters.iter().copied().enumerate() {
      if character == '_' {
         result.push('-');
         continue;
      }
      if character.is_uppercase() && index > 0 && letters[index - 1] != '_' {
         let previous_lower = letters[index - 1].is_lowercase() || letters[index - 1].is_numeric();
         let next_lower = letters
            .get(index + 1)
            .is_some_and(|next| next.is_lowercase());
         if previous_lower || next_lower {
            result.push('-');
         }
      }
      result.extend(character.to_lowercase());
   }
   result
}
