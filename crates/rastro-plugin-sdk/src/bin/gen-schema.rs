//! Writes `schema/plugin-protocol-v1.schema.json` from the connector messages and the manifest.
//! Run from anywhere in the workspace: `cargo run -p rastro-plugin-sdk --bin gen-schema`.
fn main() -> std::io::Result<()> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../schema/plugin-protocol-v1.schema.json");
    std::fs::write(path, rastro_plugin_sdk::schema::generate())
}
