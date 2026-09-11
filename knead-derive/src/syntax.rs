use proc_macro2::{
   Delimiter,
   Group,
   Literal,
   Spacing,
   TokenStream,
   TokenTree,
};
use venial::{
   Error,
   Item,
   parse_item,
};

/// Strips unsupported generic defaults and discriminant expressions before
/// parsing with venial.
pub fn parse_derive(input: TokenStream) -> Result<Item, Error> {
   let mut tokens = input.into_iter().collect::<Vec<_>>();
   let declaration = tokens.iter().position(|token| {
      matches!(token, TokenTree::Ident(name) if name == "struct" || name == "enum" || name == "union")
   });
   let Some(start) = declaration else {
      return Err(Error::new("expected a struct or enum"));
   };
   let is_enum = matches!(&tokens[start], TokenTree::Ident(name) if name == "enum");
   let generic_start = start + 2;
   if is_punct(tokens.get(generic_start), '<') {
      let mut depth = 1;
      let mut index = generic_start + 1;
      while index < tokens.len() {
         if is_punct(tokens.get(index), '=') && depth == 1 {
            let default_start = index;
            index += 1;
            while index < tokens.len() {
               if depth == 1
                  && (is_punct(tokens.get(index), ',') || is_punct(tokens.get(index), '>'))
               {
                  break;
               }
               angle_depth(&tokens, index, &mut depth);
               index += 1;
            }
            tokens.drain(default_start..index);
            index = default_start;
         }
         angle_depth(&tokens, index, &mut depth);
         if depth == 0 {
            break;
         }
         index += 1;
      }
   }
   if is_enum
      && let Some(&mut TokenTree::Group(ref mut body)) = tokens.last_mut()
      && body.delimiter() == Delimiter::Brace
   {
      let mut variants = body.stream().into_iter().collect::<Vec<_>>();
      let mut index = 0;
      while index < variants.len() {
         if is_punct(variants.get(index), '=') {
            let length = expression_end(&variants[index + 1..]);
            variants.drain(index..index + 1 + length);
         } else {
            index += 1;
         }
      }
      let mut normalized = Group::new(Delimiter::Brace, variants.into_iter().collect());
      normalized.set_span(body.span());
      *body = normalized;
   }
   parse_item(tokens.into_iter().collect())
}

/// Finds the boundary of an expression within tokens by tracking angle bracket
/// depth and closures.
pub fn expression_end(tokens: &[TokenTree]) -> usize {
   let mut depth = 0_usize;
   let mut closure = false;
   let mut operand = true;
   let mut type_context = false;
   for (index, token) in tokens.iter().enumerate() {
      match *token {
         TokenTree::Punct(ref punct) => {
            match punct.as_char() {
               ',' if depth == 0 && !closure => return index,
               '<' if depth > 0
                  || (operand && !preceded_joint(tokens, index, '<'))
                  || type_context
                  || (index >= 2
                     && is_punct(tokens.get(index - 1), ':')
                     && is_punct(tokens.get(index - 2), ':')) =>
               {
                  depth += 1;
                  operand = true;
               },
               '>' if depth > 0
                  && !is_punct(
                     index
                        .checked_sub(1)
                        .and_then(|previous| tokens.get(previous)),
                     '-',
                  ) =>
               {
                  depth -= 1;
                  operand = false;
                  type_context = false;
               },
               '|' if depth == 0 && closure => {
                  closure = false;
                  operand = true;
                  type_context = false;
               },
               '|' if depth == 0 && operand && !preceded_joint(tokens, index, '|') => {
                  closure = true;
               },
               ':' if closure => type_context = true,
               '?' => operand = false,
               '.' | ':' | '\'' => {},
               _ => operand = true,
            }
         },
         TokenTree::Ident(ref ident) => {
            type_context |= ident == "as";
            operand = matches!(
               ident.to_string().as_str(),
               "move" | "async" | "return" | "break" | "yield"
            );
         },
         TokenTree::Literal(_) | TokenTree::Group(_) => operand = false,
      }
   }
   tokens.len()
}

/// Updates the angle bracket nesting depth at the given token index.
fn angle_depth(tokens: &[TokenTree], index: usize, depth: &mut usize) {
   if is_punct(tokens.get(index), '<') {
      *depth += 1;
   } else if is_punct(tokens.get(index), '>')
      && !is_punct(
         index
            .checked_sub(1)
            .and_then(|previous| tokens.get(previous)),
         '-',
      )
   {
      *depth = depth.saturating_sub(1);
   }
}

/// Checks whether an optional token matches a specific punctuation character.
fn is_punct(token: Option<&TokenTree>, expected: char) -> bool {
   matches!(token, Some(TokenTree::Punct(punct)) if punct.as_char() == expected)
}

/// Checks whether the token preceding the index is joint punctuation matching
/// the character.
fn preceded_joint(tokens: &[TokenTree], index: usize, expected: char) -> bool {
   matches!(index.checked_sub(1).and_then(|previous| tokens.get(previous)),
      Some(TokenTree::Punct(punct)) if punct.as_char() == expected && punct.spacing() == Spacing::Joint)
}

/// Decodes a string literal token into its unescaped string value, supporting
/// raw string literals.
pub fn decode_string(literal: &Literal) -> Option<String> {
   let text = literal.to_string();
   if let Some(rest) = text.strip_prefix('r') {
      let hashes = rest.bytes().take_while(|byte| *byte == b'#').count();
      let content = rest.get(hashes..)?.strip_prefix('"')?;
      let suffix = format!("\"{}", "#".repeat(hashes));
      Some(content.strip_suffix(&suffix)?.to_owned())
   } else {
      unescape_cooked(text.strip_prefix('"')?.strip_suffix('"')?)
   }
}

/// Resolves standard Rust escape sequences in a cooked string literal,
/// returning `None` if an escape sequence is invalid.
fn unescape_cooked(input: &str) -> Option<String> {
   let mut output = String::with_capacity(input.len());
   let mut chars = input.chars();
   while let Some(next) = chars.next() {
      if next != '\\' {
         output.push(next);
         continue;
      }
      let escape = chars.next()?;
      match escape {
         'n' => output.push('\n'),
         'r' => output.push('\r'),
         't' => output.push('\t'),
         '\\' => output.push('\\'),
         '\'' => output.push('\''),
         '"' => output.push('"'),
         '0' => output.push('\0'),
         'x' => {
            let high = chars.next()?;
            let low = chars.next()?;
            let value = high.to_digit(16)? * 16 + low.to_digit(16)?;
            if value > 0x7F {
               return None;
            }
            output.push(char::from_u32(value)?);
         },
         'u' => {
            if chars.next()? != '{' {
               return None;
            }
            let mut value = 0_u32;
            let mut digits = 0_u32;
            loop {
               let digit = chars.next()?;
               if digit == '}' {
                  break;
               }
               if digit == '_' {
                  if digits == 0 {
                     return None;
                  }
                  continue;
               }
               value = value.checked_mul(16)?.checked_add(digit.to_digit(16)?)?;
               digits += 1;
               if digits > 6 {
                  return None;
               }
            }
            if digits == 0 {
               return None;
            }
            output.push(char::from_u32(value)?);
         },
         '\n' | '\r' => {
            if escape == '\r' && chars.next()? != '\n' {
               return None;
            }
            chars = chars
               .as_str()
               .trim_start_matches([' ', '\t', '\n', '\r'])
               .chars();
         },
         _ => return None,
      }
   }
   Some(output)
}
