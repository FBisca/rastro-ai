//! Writes `schema/operations-v2.schema.json` from the operation catalog.
//! Run from the repository root: `cargo run -p rastro-operations --bin gen-schema`.
fn main() -> std::io::Result<()> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../schema/operations-v2.schema.json");
    std::fs::write(path, rastro_operations::schema::generate())
}
