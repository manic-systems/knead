#![expect(clippy::expect_used, reason = "it's a test")]

use std::{
   collections::BTreeMap,
   env,
   fs,
   path::{
      Path,
      PathBuf,
   },
};

use knead::{
   ast::{
      Document,
      Literal,
      Node,
      Radix,
      Value,
   },
   dialect::Dialect,
};

#[derive(Debug, PartialEq)]
enum NormalizedValue {
   Str(String),
   Int(String),
   Dec(String),
   Bool(bool),
   Null,
}

#[derive(Debug, PartialEq)]
struct NormalizedNode {
   name:      String,
   type_name: Option<String>,
   args:      Vec<(Option<String>, NormalizedValue)>,
   props:     Vec<(String, Option<String>, NormalizedValue)>,
   children:  Vec<Self>,
}

fn norm_doc(doc: &Document) -> Vec<NormalizedNode> {
   doc.nodes.iter().map(norm_node).collect()
}

fn norm_node(node: &Node) -> NormalizedNode {
   let mut props = BTreeMap::<String, (Option<String>, NormalizedValue)>::new();
   for prop in &node.properties {
      let anno = prop
         .value
         .type_name
         .as_ref()
         .map(|spanned| spanned.value.to_string());
      let value = norm_value(&prop.value);
      props.insert(prop.name.value.to_string(), (anno, value));
   }
   NormalizedNode {
      name:      node.name.value.to_string(),
      type_name: node
         .type_name
         .as_ref()
         .map(|spanned| spanned.value.to_string()),
      args:      node
         .arguments
         .iter()
         .map(|value| {
            let anno = value
               .type_name
               .as_ref()
               .map(|spanned| spanned.value.to_string());
            (anno, norm_value(value))
         })
         .collect(),
      props:     props
         .into_iter()
         .map(|(name, (anno, value))| (name, anno, value))
         .collect(),
      children:  node.children.as_ref().map(norm_doc).unwrap_or_default(),
   }
}

fn norm_value(value: &Value) -> NormalizedValue {
   match value.literal {
      Literal::String(ref text) => NormalizedValue::Str(text.to_string()),
      Literal::Bool(flag) => NormalizedValue::Bool(flag),
      Literal::Null => NormalizedValue::Null,
      Literal::Integer(ref integer) => {
         NormalizedValue::Int(norm_int(integer.radix, &integer.digits))
      },
      Literal::Decimal(ref decimal) => NormalizedValue::Dec(norm_decimal(decimal)),
      _ => NormalizedValue::Str(format!("{value:?}")),
   }
}

fn norm_int(radix: Radix, digits: &str) -> String {
   let base = match radix {
      Radix::Binary => 2_u32,
      Radix::Octal => 8_u32,
      Radix::Decimal => 10_u32,
      Radix::Hexadecimal => 16_u32,
   };
   let clean = digits.replace('_', "");
   let (neg, mut mag) = clean.strip_prefix('-').map_or_else(
      || (false, clean.strip_prefix('+').unwrap_or(&clean)),
      |rest| (true, rest),
   );
   for prefix in ["0b", "0o", "0x", "0B", "0O", "0X"] {
      if let Some(rest) = mag.strip_prefix(prefix) {
         mag = rest;
         break;
      }
   }
   let mut dec = vec![0];
   for ch in mag.chars() {
      let msg = format!("corpus integer holds a valid digit for radix {base} in {digits}");
      let mut carry = ch.to_digit(base).expect(&msg);
      for slot in &mut dec {
         let cur = u32::from(*slot) * base + carry;
         *slot = u8::try_from(cur % 10).expect("remainder below ten");
         carry = cur / 10;
      }
      while carry > 0 {
         dec.push(u8::try_from(carry % 10).expect("remainder below ten"));
         carry /= 10;
      }
   }
   while dec.len() > 1 && dec.last() == Some(&0) {
      dec.pop();
   }
   let mut out = dec
      .iter()
      .rev()
      .map(|digit| char::from(b'0' + digit))
      .collect::<String>();
   if neg && out != "0" {
      out.insert(0, '-');
   }
   out
}

fn norm_decimal(text: &str) -> String {
   let clean = text.replace('_', "").to_ascii_lowercase();
   let hashed = clean.strip_prefix('#').unwrap_or(&clean);
   let (neg, rest) = hashed.strip_prefix('-').map_or_else(
      || (false, hashed.strip_prefix('+').unwrap_or(hashed)),
      |tail| (true, tail),
   );
   if rest == "inf" {
      return if neg {
         "-inf".to_owned()
      } else {
         "inf".to_owned()
      };
   }
   if rest == "nan" {
      return "nan".to_owned();
   }
   let (mantissa, exponent) = match rest.split_once('e') {
      Some((head, tail)) => (head, tail),
      None => (rest, "0"),
   };
   let msg = format!("corpus decimal holds a valid exponent in {text}");
   let exp_val = exponent.parse::<i64>().expect(&msg);
   let (int_part, frac_part) = match mantissa.split_once('.') {
      Some((head, tail)) => (head, tail),
      None => (mantissa, ""),
   };
   let mut digits = format!("{int_part}{frac_part}");
   digits = digits.trim_start_matches('0').to_owned();
   if digits.is_empty() {
      return if neg { "-0".to_owned() } else { "0".to_owned() };
   }
   let mut exp10 = exp_val - i64::try_from(frac_part.len()).expect("fraction length fits i64");
   while digits.ends_with('0') {
      digits.pop();
      exp10 += 1;
   }
   if digits.is_empty() {
      return if neg { "-0".to_owned() } else { "0".to_owned() };
   }
   if neg {
      format!("-{digits}e{exp10}")
   } else {
      format!("{digits}e{exp10}")
   }
}

#[test]
fn spec_corpus_v2() {
   spec_corpus(Dialect::V2, "KNEAD_SPEC_CORPUS_V2", "v2", 338);
}

#[test]
fn spec_corpus_v1() {
   spec_corpus(Dialect::V1, "KNEAD_SPEC_CORPUS_V1", "v1", 155);
}

fn spec_corpus(dialect: Dialect, variable: &str, folder: &str, expected_cases: usize) {
   let root = env::var_os(variable).map_or_else(
      || {
         PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/corpus")
            .join(folder)
      },
      PathBuf::from,
   );
   let mut names = Vec::<String>::new();
   let entries = fs::read_dir(root.join("input"))
      .map_err(|error| {
         format!(
            "no corpus at {}, run `nu tests/corpus/fetch.nu` or set {variable} ({error})",
            root.display()
         )
      })
      .expect("spec corpus");
   for entry in entries {
      let path = entry.expect("corpus entry").path();
      if path.extension().is_some_and(|ext| ext == "kdl") {
         names.push(
            path
               .file_name()
               .expect("file name")
               .to_string_lossy()
               .into(),
         );
      }
   }
   names.sort();
   assert!(
      names.len() == expected_cases,
      "expected {expected_cases} corpus cases"
   );
   let mut failures = Vec::<String>::new();
   for name in &names {
      match fs::read_to_string(root.join("input").join(name)) {
         Err(error) => failures.push(format!("{name} unreadable with {error}")),
         Ok(input) => check_case(dialect, &root, name, &input, &mut failures),
      }
   }
   assert!(
      failures.is_empty(),
      "{}\n{} of {} cases failed",
      failures.join("\n"),
      failures.len(),
      names.len()
   );
}

fn check_case(dialect: Dialect, root: &Path, name: &str, input: &str, failures: &mut Vec<String>) {
   if root.join("expected_kdl").join(name).exists() {
      match fs::read_to_string(root.join("expected_kdl").join(name)) {
         Err(error) => failures.push(format!("{name} expected unreadable with {error}")),
         Ok(want_src) => {
            match (dialect.parse(input), dialect.parse(&want_src)) {
               (Ok(got), Ok(want)) => {
                  let (got_norm, want_norm) = (norm_doc(&got), norm_doc(&want));
                  if got_norm != want_norm {
                     failures.push(format!(
                        "{name} AST differs\n got {got_norm:?}\n want {want_norm:?}"
                     ));
                  }
               },
               (Err(error), _) => failures.push(format!("{name} input rejected with {error}")),
               (_, Err(error)) => failures.push(format!("{name} expected rejected with {error}")),
            }
         },
      }
   } else {
      match dialect.parse(input) {
         Ok(document) => failures.push(format!("{name} parsed but must fail with {document:?}")),
         Err(error) => {
            let start = error.span().offset();
            let end = start + error.span().len();
            assert!(end <= input.len(), "{name} span extends beyond input");
            assert!(
               input.is_char_boundary(start),
               "{name} span starts inside a character"
            );
            assert!(
               input.is_char_boundary(end),
               "{name} span ends inside a character"
            );
         },
      }
   }
}
