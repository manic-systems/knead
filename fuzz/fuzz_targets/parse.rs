#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|source: &str| {
   let _ = knead::dialect::Dialect::V2.parse(source);
   let _ = knead::dialect::Dialect::V1.parse(source);
});
