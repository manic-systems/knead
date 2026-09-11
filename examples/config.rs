#![expect(clippy::print_stdout, reason = "this is an example")]

use std::{
   collections::BTreeMap,
   error::Error,
   net::SocketAddr,
};

use knead::decode::Decode as _;
use knead_derive::{
   Decode,
   DecodeScalar,
};

/// Speed versus safety trade-off a service runs under, decoded from a bare KDL
/// string.
#[derive(DecodeScalar)]
enum Mode {
   /// Favors throughput over safety checks.
   Fast,
   /// Favors safety checks over throughput, and is the default when the node
   /// sets no mode.
   Safe,
}

/// Log settings read from properties on the service node itself rather than a
/// child.
#[derive(Decode)]
struct Logging {
   /// Whether the service emits verbose log output, off unless the node says
   /// otherwise.
   #[knead(property, default)]
   verbose: bool,
}

/// One network service described by a single `service` node in the example
/// document.
#[derive(Decode)]
struct Service {
   /// Human readable identifier taken from the node's first argument.
   #[knead(argument)]
   name:    String,
   /// Every property not claimed by another field, kept as free-form key and
   /// value pairs.
   #[knead(properties)]
   labels:  BTreeMap<String, String>,
   /// Operating mode chosen by the `mode` property, falling back to safe.
   #[knead(property, default = Mode::Safe)]
   mode:    Mode,
   /// Socket address the service listens on, parsed from the `listen` child's
   /// argument.
   #[knead(child(name = "listen"), unwrap(argument, str))]
   address: SocketAddr,
   /// Free-form markers gathered from the argument of each `tag` child node.
   #[knead(children(name = "tag"), unwrap(argument))]
   tags:    Vec<String>,
   /// Logging options flattened into this node instead of living under a child.
   #[knead(flatten)]
   logging: Logging,
}

fn main() -> Result<(), Box<dyn Error>> {
   let document = knead::parse(
      r#"service demo mode=fast verbose=#true owner=local {
         listen "127.0.0.1:8080"
         tag web
         tag internal
      }"#,
   )?;
   let service = Service::decode_node(&document.nodes[0])?;
   let mode = match service.mode {
      Mode::Fast => "fast",
      Mode::Safe => "safe",
   };
   println!(
      "{} listens on {} in {mode} mode",
      service.name, service.address
   );
   println!("tags {}", service.tags.join(" "));
   for (key, value) in &service.labels {
      println!("{key}={value}");
   }
   println!("verbose {}", service.logging.verbose);
   Ok(())
}
