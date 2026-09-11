use std::{
   borrow::Cow,
   collections::VecDeque,
   mem,
};

use crate::{
   ast::{
      Document,
      Literal,
      Node,
      Property,
      Value,
   },
   dialect::Dialect,
   errors::Error,
   lexer::{
      Lexer,
      Tag,
      Token,
      TokenKind,
   },
   span::{
      Span,
      Spanned,
   },
};

/// Nesting limit that `parse_node` refuses to exceed so hostile input cannot
/// overflow the stack.
const MAX_DEPTH: usize = 256;

/// Parses `source` in strict mode and returns the first error instead of a
/// document.
pub fn parse(source: &str, dialect: Dialect) -> Result<Document<'_>, Error> {
   Parser::new(Lexer::new(source, dialect))?.parse_document()
}

/// Parses `source` and keeps going after errors, returning whatever document
/// could be built together with every parser and lexer error sorted by offset.
pub fn parse_lenient(source: &str, dialect: Dialect) -> (Document<'_>, Vec<Error>) {
   let mut parser = match Parser::new(Lexer::lenient(source, dialect)) {
      Ok(parser) => parser,
      Err(error) => return (Document::default(), vec![error]),
   };
   let document = match parser.parse_document() {
      Ok(document) => document,
      Err(error) => {
         parser.errors.push(error);
         Document::default()
      },
   };
   let mut errors = parser.errors;
   errors.append(&mut parser.lexer.errors);
   errors.sort_by_key(|error| error.span().offset());
   (document, errors)
}

/// One argument or property parsed from a node line before it is attached to
/// the node.
enum Entry<'src> {
   /// A positional value that is appended to the node arguments in order.
   Argument(Value<'src>),
   /// A named value whose name replaces any earlier property with the same
   /// name.
   Property(Spanned<Cow<'src, str>>, Value<'src>),
}

impl<'src> Entry<'src> {
   /// Attaches the entry to `node`, dropping any earlier property that shares
   /// the same name.
   fn push_into(self, node: &mut Node<'src>) {
      match self {
         Entry::Argument(value) => node.arguments.push(value),
         Entry::Property(name, value) => {
            node
               .properties
               .retain(|property| property.name.value != name.value);
            let span = name.span.join(value.span);
            node.properties.push(Property { name, value, span });
         },
      }
   }
}

/// A parenthesised type annotation that precedes a node name or value.
struct TypePrefix<'src> {
   /// Range covering both parentheses so the annotated item can extend its own
   /// span over it.
   span: Span,
   /// The type name found between the parentheses, with its own span.
   name: Spanned<Cow<'src, str>>,
}

/// Builds a value from a literal and an optional type prefix, widening the span
/// to include the prefix when one is present.
fn scalar<'src>(
   type_prefix: Option<TypePrefix<'src>>,
   literal: Literal<'src>,
   span: Span,
) -> Value<'src> {
   match type_prefix {
      Some(prefix) => {
         Value {
            type_name: Some(prefix.name),
            literal,
            span: prefix.span.join(span),
         }
      },
      None => {
         Value {
            type_name: None,
            literal,
            span,
         }
      },
   }
}

/// Recursive descent parser that streams tokens from the lexer with a single
/// token of state and an on demand lookahead queue.
struct Parser<'src> {
   /// Token source that also owns the dialect, the leniency flag, and any lexer
   /// errors.
   lexer:   Lexer<'src>,
   /// Token being examined, which is always valid because the lexer is primed
   /// in `new`.
   current: Token<'src>,
   /// Tokens already pulled from the lexer for lookahead but not yet consumed.
   pending: VecDeque<Token<'src>>,
   /// Diagnostics collected in lenient mode, sorted by offset before they are
   /// returned.
   errors:  Vec<Error>,
}

impl<'src> Parser<'src> {
   /// Builds a parser primed with the first token, failing if that token cannot
   /// be lexed.
   fn new(mut lexer: Lexer<'src>) -> Result<Self, Error> {
      let current = lexer.next_token()?;
      Ok(Self {
         lexer,
         current,
         pending: VecDeque::new(),
         errors: Vec::new(),
      })
   }

   /// Parses every top level node and returns a document spanning the whole
   /// source.
   fn parse_document(&mut self) -> Result<Document<'src>, Error> {
      let nodes = self.parse_nodes(0, false)?;
      Ok(Document {
         nodes,
         span: Span::new(0, self.lexer.source_len()),
      })
   }

   /// Records `error` and continues in lenient mode, or returns it in strict
   /// mode.
   fn fail(&mut self, error: Error) -> Result<(), Error> {
      if !self.lexer.is_lenient() {
         return Err(error);
      }
      self.errors.push(error);
      Ok(())
   }

   /// Skips tokens until the next node boundary at the current nesting level,
   /// or the end of input.
   fn recover(&mut self) -> Result<(), Error> {
      let mut depth = 0_usize;
      loop {
         let tag = self.current.kind.tag();
         let terminator = matches!(tag, Tag::Newline | Tag::Semicolon | Tag::CloseBrace);
         if tag == Tag::Eof || (depth == 0 && terminator) {
            return Ok(());
         }
         if tag == Tag::OpenBrace {
            depth += 1;
         } else if tag == Tag::CloseBrace {
            depth -= 1;
         }
         self.advance()?;
      }
   }

   /// Reports whether the current token has the given tag.
   fn at(&self, tag: Tag) -> bool {
      self.current.kind.tag() == tag
   }

   /// Returns the span of the current token.
   const fn span(&self) -> Span {
      self.current.span
   }

   /// Builds a syntax error located at the current token.
   fn error(&self, message: &str) -> Error {
      Error::syntax(self.span(), message)
   }

   /// Returns the tag of the token `index` positions ahead, filling the
   /// lookahead queue as needed.
   fn tag_at(&mut self, index: usize) -> Result<Tag, Error> {
      let Some(pending_index) = index.checked_sub(1) else {
         return Ok(self.current.kind.tag());
      };
      while self.pending.len() <= pending_index {
         let token = self.lexer.next_token()?;
         self.pending.push_back(token);
      }
      Ok(self.pending[pending_index].kind.tag())
   }

   /// Moves to the next token, draining the lookahead queue before asking the
   /// lexer, and returns the token that was current.
   fn advance(&mut self) -> Result<Token<'src>, Error> {
      let next = match self.pending.pop_front() {
         Some(token) => token,
         None => self.lexer.next_token()?,
      };
      Ok(mem::replace(&mut self.current, next))
   }

   /// Consumes the current token if it has the given tag, otherwise fails with
   /// `message`.
   fn expect(&mut self, tag: Tag, message: &str) -> Result<Token<'src>, Error> {
      if !self.at(tag) {
         return Err(self.error(message));
      }
      self.advance()
   }

   /// Reports whether the current token can serve as a node, property, or type
   /// name.
   fn at_name(&self) -> bool {
      self.at(Tag::String) || self.at(Tag::Identifier)
   }

   /// Consumes a string or identifier token as a name, failing with `message`
   /// otherwise.
   fn take_name(&mut self, message: &str) -> Result<Spanned<Cow<'src, str>>, Error> {
      if !self.at_name() {
         return Err(self.error(message));
      }
      let token = self.advance()?;
      if let TokenKind::String(text) | TokenKind::Identifier(text) = token.kind {
         Ok(Spanned::new(text, token.span))
      } else {
         Err(Error::syntax(token.span, message))
      }
   }

   /// Consumes consecutive space tokens and reports whether any were skipped.
   fn skip_spaces(&mut self) -> Result<bool, Error> {
      let mut skipped = false;
      while self.at(Tag::Space) {
         self.advance()?;
         skipped = true;
      }
      Ok(skipped)
   }

   /// Consumes any run of space and newline tokens.
   fn skip_line_space(&mut self) -> Result<(), Error> {
      while self.at(Tag::Space) || self.at(Tag::Newline) {
         self.advance()?;
      }
      Ok(())
   }

   /// Consumes a `/-` token and the whitespace after it, reporting any stacked
   /// `/-` that follows.
   fn skip_slashdash(&mut self) -> Result<(), Error> {
      self.expect(Tag::Slashdash, "expected '/-'")?;
      self.skip_line_space()?;
      while self.at(Tag::Slashdash) {
         self.fail(self.error("unexpected '/-'"))?;
         self.advance()?;
         self.skip_line_space()?;
      }
      Ok(())
   }

   /// Looks past the current `/-` and whitespace to tell whether it hides a
   /// children block.
   fn slashdash_hides_children(&mut self) -> Result<bool, Error> {
      let mut index = 1;
      while matches!(self.tag_at(index)?, Tag::Space | Tag::Newline) {
         index += 1;
      }
      Ok(self.tag_at(index)? == Tag::OpenBrace)
   }

   /// Parses nodes until end of input or, when `nested`, the closing brace,
   /// recovering after each failed node in lenient mode.
   fn parse_nodes(&mut self, depth: usize, nested: bool) -> Result<Vec<Node<'src>>, Error> {
      let mut nodes = Vec::new();
      loop {
         self.skip_line_space()?;
         let tag = self.current.kind.tag();
         if tag == Tag::Eof {
            if nested {
               self.fail(self.error("unexpected end of input"))?;
            }
            return Ok(nodes);
         }
         if tag == Tag::CloseBrace && nested {
            return Ok(nodes);
         }
         if tag == Tag::CloseBrace || tag == Tag::Semicolon {
            let message = if tag == Tag::Semicolon {
               "unexpected ';'"
            } else {
               "unexpected '}'"
            };
            self.fail(self.error(message))?;
            self.advance()?;
            continue;
         }
         match self.parse_node(depth, nested) {
            Ok(Some(node)) => nodes.push(node),
            Ok(None) => {},
            Err(error) => {
               self.fail(error)?;
               self.recover()?;
            },
         }
      }
   }

   /// Parses one node and returns `None` when it was hidden by a slashdash or
   /// `Err` when `depth` exceeds `MAX_DEPTH`.
   fn parse_node(&mut self, depth: usize, nested: bool) -> Result<Option<Node<'src>>, Error> {
      if depth > MAX_DEPTH {
         return Err(self.error("maximum nesting depth exceeded"));
      }
      if self.at(Tag::Slashdash) {
         self.skip_slashdash()?;
         self.parse_node_inner(depth, nested)?;
         return Ok(None);
      }
      self.parse_node_inner(depth, nested).map(Some)
   }

   /// Parses a node from its optional type annotation through its terminator,
   /// then widens its span over every argument, property, and children
   /// block.
   fn parse_node_inner(&mut self, depth: usize, nested: bool) -> Result<Node<'src>, Error> {
      let prefix = self.parse_type_prefix()?;
      let name = self.take_name("expected node name")?;
      let (span, type_name) = prefix.map_or((name.span, None), |annotation| {
         (annotation.span.join(name.span), Some(annotation.name))
      });
      let mut node = Node {
         span,
         type_name,
         name,
         arguments: Vec::new(),
         properties: Vec::new(),
         children: None,
      };
      if let Err(error) = self.parse_entries(&mut node) {
         self.fail(error)?;
         self.recover()?;
      }
      let mut blocks = 0_usize;
      loop {
         self.skip_spaces()?;
         let hidden = self.at(Tag::Slashdash) && self.slashdash_hides_children()?;
         if !hidden && !self.at(Tag::OpenBrace) {
            break;
         }
         if hidden {
            self.skip_slashdash()?;
         }
         let children = self.parse_children(depth)?;
         blocks += 1;
         let one_block_only = self.lexer.dialect() == Dialect::V1;
         if (!hidden && node.children.is_some()) || (one_block_only && blocks > 1) {
            self.fail(Error::syntax(children.span, "unexpected children block"))?;
         } else if !hidden {
            node.children = Some(children);
         }
      }
      self.skip_spaces()?;
      let tag = self.current.kind.tag();
      if matches!(tag, Tag::Newline | Tag::Semicolon) {
         self.advance()?;
      } else if tag == Tag::CloseBrace && !nested {
         self.fail(self.error("unexpected '}'"))?;
         self.advance()?;
      } else if !matches!(tag, Tag::Eof | Tag::CloseBrace) {
         self.fail(self.error("expected node terminator"))?;
         self.recover()?;
      }
      for value in &node.arguments {
         node.span = node.span.join(value.span);
      }
      for property in &node.properties {
         node.span = node.span.join(property.span);
      }
      if let Some(children) = node.children.as_ref() {
         node.span = node.span.join(children.span);
      }
      Ok(node)
   }

   /// Parses arguments and properties into `node` until something that is not
   /// an entry is reached.
   fn parse_entries(&mut self, node: &mut Node<'src>) -> Result<(), Error> {
      loop {
         let separated = self.skip_spaces()?;
         if self.at(Tag::Slashdash) {
            if self.slashdash_hides_children()? {
               return Ok(());
            }
            self.skip_slashdash()?;
            self.parse_entry()?;
            continue;
         }
         let starts_entry = self.at(Tag::OpenParen) || self.at_name() || self.at(Tag::Literal);
         if !separated || !starts_entry {
            return Ok(());
         }
         self.parse_entry()?.push_into(node);
      }
   }

   /// Parses one argument or property, treating a bare identifier without `=`
   /// as an error in v1.
   fn parse_entry(&mut self) -> Result<Entry<'src>, Error> {
      let prefix = self.parse_type_prefix()?;
      if !self.at_name() {
         let literal = self.take_literal("expected argument or property")?;
         return Ok(Entry::Argument(scalar(prefix, literal.value, literal.span)));
      }
      let bare = self.at(Tag::Identifier);
      let name = self.take_name("expected value")?;
      let mut equals_index = 0;
      while self.lexer.dialect() == Dialect::V2 && self.tag_at(equals_index)? == Tag::Space {
         equals_index += 1;
      }
      if self.tag_at(equals_index)? != Tag::Equals {
         if bare && self.lexer.dialect() == Dialect::V1 {
            return Err(Error::syntax(name.span, "bare identifier is not a value"));
         }
         return Ok(Entry::Argument(scalar(
            prefix,
            Literal::String(name.value),
            name.span,
         )));
      }
      for _ in 0..equals_index {
         self.advance()?;
      }
      if let Some(annotation) = prefix {
         self.fail(Error::syntax(
            annotation.span,
            "property names cannot have type annotations",
         ))?;
      }
      self.advance()?;
      let value = self.parse_value()?;
      Ok(Entry::Property(name, value))
   }

   /// Consumes the current token as a literal value, accepting bare identifiers
   /// only in v2.
   fn take_literal(&mut self, message: &str) -> Result<Spanned<Literal<'src>>, Error> {
      let span = self.span();
      match self.advance()?.kind {
         TokenKind::String(text) => Ok(Spanned::new(Literal::String(text), span)),
         TokenKind::Identifier(text) if self.lexer.dialect() == Dialect::V2 => {
            Ok(Spanned::new(Literal::String(text), span))
         },
         TokenKind::Identifier(_) => Err(Error::syntax(span, "bare identifier is not a value")),
         TokenKind::Literal(literal) => Ok(Spanned::new(literal, span)),
         TokenKind::Space
         | TokenKind::Newline
         | TokenKind::OpenParen
         | TokenKind::CloseParen
         | TokenKind::OpenBrace
         | TokenKind::CloseBrace
         | TokenKind::Equals
         | TokenKind::Semicolon
         | TokenKind::Slashdash
         | TokenKind::Eof => Err(Error::syntax(span, message)),
      }
   }

   /// Parses the value on the right hand side of a property.
   fn parse_value(&mut self) -> Result<Value<'src>, Error> {
      self.skip_spaces()?;
      let prefix = self.parse_type_prefix()?;
      let literal = self.take_literal("expected value")?;
      Ok(scalar(prefix, literal.value, literal.span))
   }

   /// Parses a parenthesised type annotation if one starts at the current
   /// token.
   fn parse_type_prefix(&mut self) -> Result<Option<TypePrefix<'src>>, Error> {
      if !self.at(Tag::OpenParen) {
         return Ok(None);
      }
      let open = self.advance()?.span;
      self.skip_spaces()?;
      let name = self.take_name("expected type name")?;
      self.skip_spaces()?;
      let close = self.expect(Tag::CloseParen, "expected ')'")?.span;
      self.skip_spaces()?;
      Ok(Some(TypePrefix {
         span: open.join(close),
         name,
      }))
   }

   /// Parses a braced children block, tolerating end of input where the closing
   /// brace should be.
   fn parse_children(&mut self, depth: usize) -> Result<Document<'src>, Error> {
      let open = self.expect(Tag::OpenBrace, "expected '{'")?.span;
      let nodes = self.parse_nodes(depth + 1, true)?;
      let close = if self.at(Tag::Eof) {
         self.span()
      } else {
         self.expect(Tag::CloseBrace, "expected '}'")?.span
      };
      Ok(Document {
         nodes,
         span: open.join(close),
      })
   }
}
