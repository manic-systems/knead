#![no_main]
#![expect(dead_code, reason = "decoded fields are never read")]

use std::{
   collections::BTreeMap,
   path::PathBuf,
};

use knead::{
   ast::{
      Node,
      Value,
   },
   decode::{
      Decode,
      DecodeScalar,
   },
};
use knead_derive::Decode;
use libfuzzer_sys::fuzz_target;

#[derive(Decode)]
struct Tree {
   #[knead(node_name)]
   name:   String,
   #[knead(argument, default)]
   head:   Option<String>,
   #[knead(arguments)]
   tail:   Vec<i64>,
   #[knead(properties)]
   labels: BTreeMap<String, f64>,
   #[knead(child(name = "child"))]
   child:  Option<Box<Tree>>,
   #[knead(children)]
   rest:   Vec<Tree>,
}

fn scalars(value: &Value<'_>) {
   let _ = i128::decode(value);
   let _ = u8::decode(value);
   let _ = f64::decode(value);
   let _ = bool::decode(value);
   let _ = String::decode(value);
   let _ = PathBuf::decode(value);
   let _ = Option::<u32>::decode(value);
   let _ = value.parse_str::<std::net::IpAddr>();
}

fn walk(node: &Node<'_>) {
   let _ = Tree::decode_node(node);
   for value in &node.arguments {
      scalars(value);
   }
   for property in &node.properties {
      scalars(&property.value);
   }
   for child in node.child_nodes() {
      walk(child);
   }
}

fuzz_target!(|source: &str| {
   let Ok(document) = knead::parse(source) else {
      return;
   };
   for node in document.nodes() {
      walk(node);
   }
});
