//! The operation catalog (ADR 0007): every operation Rastro offers is defined here once, with its
//! name, description, inputs, defaults, result and the surfaces that expose it. The CLI parses
//! and dispatches through it; the committed schema is generated from it.
pub mod catalog;
pub mod envelope;
pub mod read;
pub mod requests;
pub mod results;
pub mod schema;

pub use catalog::{Input, OperationSpec, Surface};
pub use requests::{Operation, Request};
pub use results::Response;
