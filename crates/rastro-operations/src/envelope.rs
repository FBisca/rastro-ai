//! The `--json` envelope, built in one place: a schema version, the operation, and exactly one
//! of `result` or `error`.
use serde::Serialize;
use serde_json::{Value, json};
use std::fmt::Display;

pub const SCHEMA_VERSION: u32 = 2;

pub fn success(operation: &str, result: &impl Serialize) -> Value {
    json!({"schema_version": SCHEMA_VERSION, "operation": operation, "result": result})
}
pub fn failure(operation: &str, message: impl Display) -> Value {
    json!({"schema_version": SCHEMA_VERSION, "operation": operation, "error": message.to_string()})
}
/// A command line that did not parse.
pub fn usage_error(operation: &str, message: impl Display) -> Value {
    failure(operation, format!("usage: {message}"))
}
