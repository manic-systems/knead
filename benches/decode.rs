#![expect(dead_code, reason = "decoded fields are never read")]
#![expect(clippy::unwrap_used, reason = "benching")]

use std::hint::black_box;

use criterion::{
   Criterion,
   criterion_group,
   criterion_main,
};
use knead::{
   ast::Value,
   decode::{
      Decode as _,
      DecodeScalar,
   },
   errors::Error,
};
use knead_derive::Decode;

/// Reverse-proxy policy for a Forgejo instance that every section type decodes
/// from.
const FORGEJO: &str = include_str!("inputs/forgejo.kdl");

/// Script snippet kept as an unparsed string, standing in for a real Rhai
/// expression.
#[derive(Debug)]
struct Rhai(String);

impl DecodeScalar for Rhai {
   fn type_check(_value: &Value<'_>) -> Result<(), Error> {
      Ok(())
   }

   fn decode(value: &Value<'_>) -> Result<Self, Error> {
      value.expect_string().map(|text| Self(text.to_owned()))
   }
}

/// Top-level node of the policy, with one variant per configuration area.
#[derive(Debug, Decode)]
enum Section {
   /// Named address sets built from remote IP range feeds and ASN lists.
   Networks(Networks),
   /// Named predicates that rule and scorecard scripts reference by name.
   Conditions(Conditions),
   /// Search engine bots verified by the hostname suffixes their addresses
   /// resolve to.
   Crawlers(Crawlers),
   /// Cookie and proof-of-work checks a rule can demand before letting a
   /// request through.
   Challenges(Challenges),
   /// Tarpit destinations that scrapers get sent into.
   Mazes(Mazes),
   /// Ordered request matchers that each pick an action.
   Rules(Rules),
   /// Weighted signals and thresholds that grade a client over time.
   Scoring(Scoring),
}

/// Section grouping every named network under one node.
#[derive(Debug, Decode)]
struct Networks {
   /// Each `network` child, one per named address set.
   #[knead(children(name = "network"))]
   networks: Vec<Network>,
}

/// Named set of IP ranges built from remote JSON feeds and ASN numbers.
#[derive(Debug, Decode)]
struct Network {
   /// Identifier that rule and signal scripts use to look the set up.
   #[knead(argument)]
   name: String,
   /// Remote feeds whose prefixes get merged into the set.
   #[knead(children(name = "url"))]
   urls: Vec<Url>,
   /// Autonomous system numbers whose announced ranges join the set.
   #[knead(children(name = "asn"), unwrap(argument))]
   asns: Vec<u32>,
}

/// Remote JSON feed together with the query that extracts prefixes from it.
#[derive(Debug, Decode)]
struct Url {
   /// Address of the feed to download.
   #[knead(argument)]
   target: String,
   /// Name of the extraction tool applied to the response, always `jq` in the
   /// sample.
   #[knead(property)]
   filter: String,
   /// Query that pulls the prefix strings out of the downloaded JSON.
   #[knead(property)]
   jq:     String,
}

/// Section grouping every named condition under one node.
#[derive(Debug, Decode)]
struct Conditions {
   /// Each `condition` child, one per reusable predicate.
   #[knead(children(name = "condition"))]
   conditions: Vec<Condition>,
}

/// Reusable predicate that other scripts invoke by wrapping its name in
/// parentheses.
#[derive(Debug, Decode)]
struct Condition {
   /// Identifier other scripts use to invoke this predicate.
   #[knead(argument)]
   name: String,
   /// Script that evaluates to whether the request matches.
   #[knead(property)]
   expr: Rhai,
}

/// Section grouping every known crawler under one node.
#[derive(Debug, Decode)]
struct Crawlers {
   /// Each `crawler` child, one per search engine bot.
   #[knead(children(name = "crawler"))]
   crawlers: Vec<Crawler>,
}

/// Search engine bot verified through the hostnames its addresses resolve to.
#[derive(Debug, Decode)]
struct Crawler {
   /// Identifier for the bot operator.
   #[knead(argument)]
   name:     String,
   /// Hostname suffixes a reverse DNS lookup must end with for the bot to count
   /// as verified.
   #[knead(child(name = "suffixes"), unwrap(arguments))]
   suffixes: Vec<String>,
}

/// Section grouping every challenge under one node.
#[derive(Debug, Decode)]
struct Challenges {
   /// Each `challenge` child, one per client verification method.
   #[knead(children(name = "challenge"))]
   challenges: Vec<Challenge>,
}

/// Client verification method that rules and thresholds can demand before
/// passing a request.
#[derive(Debug, Decode)]
struct Challenge {
   /// Identifier that rules and thresholds list to require this challenge.
   #[knead(argument)]
   name:       String,
   /// Mechanism that performs the check, such as a cookie or a proof-of-work
   /// hash.
   #[knead(property)]
   runtime:    String,
   /// Work factor for proof-of-work runtimes, absent for runtimes that need
   /// none.
   #[knead(property, default)]
   difficulty: Option<u32>,
   /// Seconds a passed challenge stays valid for the client.
   #[knead(property)]
   duration:   u64,
}

/// Section grouping every maze under one node.
#[derive(Debug, Decode)]
struct Mazes {
   /// Each `maze` child, one per tarpit destination.
   #[knead(children(name = "maze"))]
   mazes: Vec<Maze>,
}

/// Tarpit destination that feeds scrapers an endless supply of fake pages.
#[derive(Debug, Decode)]
struct Maze {
   /// Identifier that rules and thresholds use to pick this maze.
   #[knead(argument)]
   name: String,
}

/// Section grouping every request rule under one node.
#[derive(Debug, Decode)]
struct Rules {
   /// Each `rule` child in the order they are evaluated.
   #[knead(children(name = "rule"))]
   rules: Vec<Rule>,
}

/// Request matcher paired with the action to take when it fires.
#[derive(Debug, Decode)]
struct Rule {
   /// Identifier for the rule, used in logs and reports.
   #[knead(argument)]
   name:            String,
   /// Inline predicate given as a property, for short one-line matches.
   #[knead(property, default)]
   condition:       Option<Rhai>,
   /// Outcome for a matching request, such as `pass`, `deny`, `challenge`,
   /// `tarpit`, `smear` or `report`.
   #[knead(property)]
   action:          String,
   /// Status code returned when the action is a denial, absent to use the
   /// default.
   #[knead(property(name = "http-code"), default)]
   http_code:       Option<u16>,
   /// Category label attached by `report` actions so downstream stats can group
   /// requests.
   #[knead(property, default)]
   kind:            Option<String>,
   /// Which maze a `tarpit` action sends the client into, absent for other
   /// actions.
   #[knead(property, default)]
   maze:            Option<String>,
   /// Multi-line predicate given as a child node instead of a property, for
   /// long matches.
   #[knead(child(name = "condition"), unwrap(argument))]
   condition_block: Option<Rhai>,
   /// Challenge names a `challenge` or `check` action requires the client to
   /// satisfy.
   #[knead(child(name = "challenges"), unwrap(arguments), default)]
   challenges:      Vec<String>,
}

/// Section that grades clients by accumulating weighted signals against
/// thresholds.
#[derive(Debug, Decode)]
struct Scoring {
   /// Number of clients whose request rates are tracked at once.
   #[knead(property(name = "rate-capacity"))]
   rate_capacity: u32,
   /// Each `scorecard` child, one per group of requests scored together.
   #[knead(children(name = "scorecard"))]
   scorecards:    Vec<Scorecard>,
}

/// Set of weighted signals and the thresholds their combined score is checked
/// against.
#[derive(Debug, Decode)]
struct Scorecard {
   /// Label identifying this scorecard in reports.
   #[knead(argument)]
   name:       String,
   /// Predicate selecting which requests this scorecard grades.
   #[knead(property)]
   condition:  Rhai,
   /// Whether threshold actions are enforced or only recorded.
   #[knead(property)]
   mode:       String,
   /// Each `signal` child contributing weight to the score.
   #[knead(children(name = "signal"))]
   signals:    Vec<Signal>,
   /// Each `threshold` child mapping a score level to an action.
   #[knead(children(name = "threshold"))]
   thresholds: Vec<Threshold>,
}

/// Observation about a client that adds a fixed weight to the score when it
/// holds.
#[derive(Debug, Decode)]
struct Signal {
   /// Label identifying this signal in reports.
   #[knead(argument)]
   name:      String,
   /// Predicate that decides whether the signal applies to the request.
   #[knead(property)]
   condition: Rhai,
   /// Points added to the client's score while the condition holds.
   #[knead(property)]
   weight:    u32,
}

/// Score level that triggers an action once a client's total reaches it.
#[derive(Debug, Decode)]
struct Threshold {
   /// Minimum total score at which this threshold fires.
   #[knead(argument)]
   score:      u32,
   /// Outcome applied to the client at this score, such as `challenge` or
   /// `tarpit`.
   #[knead(property)]
   action:     String,
   /// Which maze a `tarpit` action sends the client into, absent for other
   /// actions.
   #[knead(property, default)]
   maze:       Option<String>,
   /// Challenge names a `challenge` action requires the client to satisfy.
   #[knead(child(name = "challenges"), unwrap(arguments), default)]
   challenges: Vec<String>,
}

/// Criterion benchmark that decodes every section of the parsed policy on each
/// iteration.
fn decode(criterion: &mut Criterion) {
   let document = knead::parse(FORGEJO).unwrap();
   for node in document.nodes() {
      Section::decode_node(node).unwrap();
   }
   assert_eq!(
      document.nodes().len(),
      7,
      "forgejo policy has seven sections"
   );
   criterion.bench_function("decode/forgejo", |bencher| {
      bencher.iter(|| {
         black_box(&document)
            .nodes()
            .iter()
            .map(|node| Section::decode_node(node).unwrap())
            .collect::<Vec<Section>>()
      });
   });
}

criterion_group!(benches, decode);
criterion_main!(benches);
