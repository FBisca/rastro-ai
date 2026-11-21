//! The one pattern every command's human output follows.
//!
//! A record is a **block**: its quote or header line first, then one labelled field per line,
//! indented two spaces. One blank line separates records; no line packs fields with pipes.
//! Coverage of two sources reads:
//!
//! ```text
//! project
//!   Scope: personal
//!   Coverage: found
//!   Last checked: 2026-11-21 14:22 UTC
//!   Fresh until: 2026-11-21 15:22 UTC
//!   Window: [2026-08-23 14:22 UTC, 2026-11-21 14:22 UTC) found, Selection: all
//!
//! other
//!   Scope: work
//!   Coverage: never looked
//! ```
//!
//! A new command renders through this module:
//!
//! - One record, one [`Block`]: [`Block::new`] carries the quote or the record's name, then one
//!   [`Block::field`] per labelled value, in the order the owner reads them. A value may be
//!   composite (a span, a tally); a value that may be absent goes through [`Block::maybe`].
//! - Every timestamp goes through [`time`]: `YYYY-MM-DD HH:MM UTC`, minute precision, the zone
//!   explicit. The machine path never comes through here: `--json` keeps full RFC 3339 times.
//! - A result set is [`records`]: the blocks joined with one blank line between them.
//! - An empty read is [`empty`]: one sentence starting `No ` that names what is absent.
//! - A write that changed nothing ends [`nothing_recorded`]: `; nothing recorded`.
//! - Source text stays escaped: route names, quotes and reasons through `display_safe` or
//!   `display_escaped` before they become a header or a field value.
//! - Plain text only: no colour, no TTY branch, so piped output reads exactly like the terminal.
//! - The style never moves the machine contract: `--json` envelopes, exit codes, stderr and the
//!   queue's shown reports stay exactly as they are.

use chrono::{DateTime, Utc};
use std::fmt;

/// The one human timestamp: `YYYY-MM-DD HH:MM UTC` at minute precision, the zone explicit.
pub(crate) fn time(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%d %H:%M UTC").to_string()
}

/// One record block under construction: its header line, then one labelled field per line.
pub(crate) struct Block {
    header: String,
    fields: Vec<String>,
}

impl Block {
    /// Start a block with its header: the record's quote or its name, already escaped.
    pub(crate) fn new(header: impl Into<String>) -> Self {
        Self {
            header: header.into(),
            fields: Vec::new(),
        }
    }

    /// Add one labelled field on its own indented line.
    pub(crate) fn field(mut self, label: &str, value: impl fmt::Display) -> Self {
        self.fields.push(format!("  {label}: {value}"));
        self
    }

    /// Add a field only when the value is present; an absent value prints no line.
    pub(crate) fn maybe<T: fmt::Display>(self, label: &str, value: Option<T>) -> Self {
        match value {
            Some(value) => self.field(label, value),
            None => self,
        }
    }
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.header)?;
        for field in &self.fields {
            f.write_str("\n")?;
            f.write_str(field)?;
        }
        Ok(())
    }
}

/// A result set: the blocks joined with one blank line between records.
pub(crate) fn records(blocks: impl IntoIterator<Item = Block>) -> String {
    blocks
        .into_iter()
        .map(|block| block.to_string())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The one empty-read voice: a sentence starting `No ` that names what is absent.
pub(crate) fn empty(what: &str) -> String {
    format!("No {what}")
}

/// The one no-op write voice: the message, ended with `; nothing recorded`.
pub(crate) fn nothing_recorded(message: &str) -> String {
    format!("{message}; nothing recorded")
}
