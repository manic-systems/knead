# knead

knead is a KDL parser and typed decoder. The parser handles both KDL v2 and
v1 using only the standard library, and the companion `knead-derive` crate
generates decoders so a config file can land directly in your own structs.

```rust
use knead::decode::Decode;
use knead_derive::Decode;

#[derive(Decode)]
struct Server {
   #[knead(argument)]
   name: String,
   #[knead(property, default = 8080)]
   port: u16,
   #[knead(child(name = "bind"), unwrap(argument))]
   address: String,
}

let document = knead::parse(r#"server demo port=9000 { bind "127.0.0.1" }"#)?;
let server = Server::decode_node(&document.nodes[0])?;
```

## Dialects

`knead::parse` reads KDL v2, and `Dialect::V1.parse` reads KDL 1.0 for
documents written against the older spec. The v1 grammar differs in a handful
of ways that matter to a parser. `true`, `false`, and `null` are bare keywords
rather than `#true` and friends, raw strings are spelled `r#"..."#`, quoted
strings are allowed to run across lines, and a bare identifier can only ever be
a name, never a value. Nothing from v2 that came later exists there, so there
are no `"""` strings, no `#` keywords, and no `\s` escapes.

Because the two grammars disagree about the same bytes, the dialect is always
an explicit choice and is never inferred from the input. `node true` is a
boolean in v1 and a syntax error in v2, and there is no reliable way to tell
which one a file meant. A derived decoder can carry that choice with
`#[knead(dialect = "v1")]`, which sets `Decode::DIALECT` on the type so callers
can parse with the dialect the type was written for.

## Errors

`parse` stops at the first problem and returns it. When you would rather see
everything wrong with a file at once, `parse_lenient` records the error, skips
ahead to the next node boundary at the same nesting depth, and carries on, then
returns whatever it managed to build along with every error it found in source
order. Recovery also works inside strings, where an invalid escape or a
disallowed character is recorded and skipped rather than ending the string, and
an unterminated single-line string is closed at the newline. A lenient document
that came back with errors is a best effort and may be missing nodes or
entries, so treat the error list as authoritative.

Every error carries a byte span, an `ErrorKind`, and a message, and implements
`std::error::Error`. With the `miette` feature enabled, `diagnostic::Diagnostic`
pairs an error with its source text so miette can render it with the offending
line highlighted.

## AST

Parsing produces an `ast::Document` that borrows from the source text rather
than copying it. Strings, names, and type annotations are `Cow<str>` slices of
the input, and they only allocate when an escape sequence or a multi-line
dedent means the decoded text no longer appears verbatim in the file. Nodes
keep their arguments, properties, children, type annotations, and spans, while
integers keep their original radix and digit string and decimals stay as text
until a decoder asks for a number. Comments and whitespace are not retained.
Nesting is capped at 256 levels, and malformed input of any kind comes back as
an error rather than a panic.

## Decoding

`Decode` can be derived for named structs, tuple structs, and enums that
dispatch on the node name, and a tuple struct with a single unannotated field
simply delegates to that field. `DecodeScalar` can be derived for enums with
unit variants. In both cases Rust names are converted to kebab case to match
the KDL.

| Attribute              | Decodes                                |
| ---------------------- | -------------------------------------- |
| `argument`             | The next argument                      |
| `arguments`            | Remaining arguments                    |
| `property`             | A property matching the field name     |
| `properties`           | Remaining properties into a map        |
| `child`                | One child matching the field name      |
| `children`             | Children, optionally filtered by name  |
| `node_name`            | The node name                          |
| `flatten`              | Fields decoded through the same cursor |
| `str`                  | A string through `FromStr`             |
| `default`              | `Default::default()` when absent       |
| `default = expression` | An expression evaluated when absent    |

`property`, `child`, and `children` accept `name = "..."` when the KDL name
differs from the field. Child fields can also reach into the child with
`unwrap(argument)`, `unwrap(arguments)`, `unwrap(properties)`, or
`unwrap(argument, str)` instead of decoding it as a struct of its own. A
missing `Option` child decodes to `None`, but a child that is present still has
to satisfy its inner type. Unknown fields, unused arguments, duplicate children,
and type annotations that disagree with the Rust type are all reported as
errors rather than ignored.

When the derive is not enough, a manual decoder implements `Decode::decode`
against a `Decoder`, which returns arguments in order and marks each property
and child as used when it is requested, so that `decode_node` can fail if
anything in the node went unclaimed. Scalar decoders implement
`DecodeScalar::decode` on an `ast::Value`, and can override `type_check` when
they want to accept a type annotation.
