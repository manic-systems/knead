#![expect(clippy::unwrap_used, reason = "benching")]

use std::hint::black_box;

use criterion::{
   Criterion,
   Throughput,
   criterion_group,
   criterion_main,
};

/// Reverse-proxy policy for a Forgejo instance, the larger of the two sample
/// documents.
const FORGEJO: &str = include_str!("inputs/forgejo.kdl");
/// Server configuration for the bagel proxy itself, the smaller sample
/// document.
const BAGEL: &str = include_str!("inputs/bagel.kdl");

/// Criterion benchmark that parses each sample document and reports throughput
/// in bytes.
fn parse(criterion: &mut Criterion) {
   let mut group = criterion.benchmark_group("parse");
   for (name, source) in [("forgejo", FORGEJO), ("bagel", BAGEL)] {
      group.throughput(Throughput::Bytes(u64::try_from(source.len()).unwrap()));
      group.bench_function(name, |bencher| {
         bencher.iter(|| knead::parse(black_box(source)).unwrap());
      });
   }
   let repeated = FORGEJO.repeat(64);
   group.throughput(Throughput::Bytes(u64::try_from(repeated.len()).unwrap()));
   group.bench_function("forgejo_x64", |bencher| {
      bencher.iter(|| knead::parse(black_box(&repeated)).unwrap());
   });
   group.finish();
}

criterion_group!(benches, parse);
criterion_main!(benches);
