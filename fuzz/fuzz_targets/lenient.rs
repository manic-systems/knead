#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|source: &str| {
   for dialect in [knead::dialect::Dialect::V2, knead::dialect::Dialect::V1] {
      check(dialect, source);
   }
});

fn check(dialect: knead::dialect::Dialect, source: &str) {
   let strict = dialect.parse(source);
   let (document, errors) = dialect.parse_lenient(source);
   match strict {
      Ok(expected) => {
         assert!(
            errors.is_empty(),
            "lenient reported {errors:?} on a valid document"
         );
         assert_eq!(expected, document);
      },
      Err(_) => {
         assert!(
            !errors.is_empty(),
            "lenient reported nothing on an invalid document"
         )
      },
   }
   for error in &errors {
      let span = error.span();
      assert!(span.end() <= source.len());
      assert!(source.is_char_boundary(span.offset()) && source.is_char_boundary(span.end()));
   }
}
