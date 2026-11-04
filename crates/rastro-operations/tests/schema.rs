#![allow(non_snake_case)]
//! The committed v2 schema is the catalog's output, byte for byte.

/// S1: the generator's output equals the committed file; any difference fails
#[test]
fn S1_generated_schema_equals_the_committed_file() {
    let generated = rastro_operations::schema::generate();
    let committed = include_str!("../../../schema/operations-v2.schema.json");
    assert!(
        generated == committed,
        "schema/operations-v2.schema.json drifted from the catalog; run \
         `cargo run -p rastro-operations --bin gen-schema` and commit the result"
    );
}
