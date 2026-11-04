//! Search: options, the shape of an answer, and the excerpt that shows why a hit matched.
use crate::{Coverage, Hit, TrustClass};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Serialize;

/// Hits a search returns when the caller names no limit.
pub const DEFAULT_SEARCH_LIMIT: usize = 20;
/// Most hits a search may be asked for.
pub const MAX_SEARCH_LIMIT: usize = 50;
/// Most bytes of an excerpt.
pub const EXCERPT_BYTES: usize = 400;

/// How a search is run.
#[derive(Clone, Copy, Debug)]
pub struct SearchOptions {
    /// Every observation, not only the latest at each location.
    pub history: bool,
    /// Most hits to return; the caller keeps it within `1..=MAX_SEARCH_LIMIT`.
    pub limit: usize,
    /// Return the whole quote and the original on each hit.
    pub full: bool,
}
impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            history: false,
            limit: DEFAULT_SEARCH_LIMIT,
            full: false,
        }
    }
}

/// The FTS5 query for free text: each word quoted whole; `None` when there is no word.
pub(crate) fn search_query(text: &str) -> Option<String> {
    let terms: Vec<_> = text
        .split_whitespace()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

/// Where in the span a hit matched: a window of at most [`EXCERPT_BYTES`] bytes.
#[derive(Debug, JsonSchema, Serialize)]
pub struct Match {
    /// Byte offset of the excerpt in the quote's span, in the same origin as the hit's `start`.
    pub start: u64,
    /// Byte offset just past the excerpt, in the same origin as the hit's `end`.
    pub end: u64,
    /// The excerpt, as stored; never rewritten.
    pub text: String,
    /// Text of the span precedes the excerpt.
    pub clipped_before: bool,
    /// Text of the span follows the excerpt.
    pub clipped_after: bool,
    /// False when the matching word couldn't be found; the excerpt then starts at the span's start.
    pub located: bool,
}

/// The excerpt of `quote` (the whole span, starting at byte `origin`) around the first match,
/// `matched` being its byte range in `quote`. Without a usable range the excerpt is the
/// span's start, `located: false`.
pub fn excerpt(quote: &str, matched: Option<(usize, usize)>, origin: u64) -> Match {
    let len = quote.len();
    let located = matched.filter(|&(s, e)| {
        s < e && e <= len && quote.is_char_boundary(s) && quote.is_char_boundary(e)
    });
    let (start, end) = match located {
        Some((s, e)) if e - s < EXCERPT_BYTES => {
            // Equal room either side of the match, moved inward at the span's ends.
            let room = EXCERPT_BYTES - (e - s);
            let start = s.saturating_sub(room / 2);
            let end = (start + EXCERPT_BYTES).min(len);
            let start = end.saturating_sub(EXCERPT_BYTES).min(start);
            // The match's own edges are boundaries, so snapping inward keeps it inside.
            (ceil_boundary(quote, start), floor_boundary(quote, end))
        }
        Some((s, _)) => (s, floor_boundary(quote, s + EXCERPT_BYTES)),
        None => (0, floor_boundary(quote, EXCERPT_BYTES.min(len))),
    };
    Match {
        start: origin + start as u64,
        end: origin + end as u64,
        text: quote[start..end].to_string(),
        clipped_before: start > 0,
        clipped_after: end < len,
        located: located.is_some(),
    }
}
fn floor_boundary(text: &str, mut at: usize) -> usize {
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}
fn ceil_boundary(text: &str, mut at: usize) -> usize {
    while !text.is_char_boundary(at) {
        at += 1;
    }
    at
}

/// One hit of a search: the citation, why it matched, and the whole quote only on request.
#[derive(Debug, JsonSchema, Serialize)]
pub struct SearchResult {
    pub id: i64,
    pub source: String,
    pub scope: crate::Scope,
    #[schemars(length(min = 1))]
    pub location: String,
    #[schemars(length(min = 1))]
    pub field: String,
    pub start: u64,
    pub end: u64,
    #[serde(rename = "match")]
    pub matched: Match,
    #[schemars(length(min = 1))]
    /// The whole span; only when the search asked for `full`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    pub trust_class: TrustClass,
    pub captured_at: DateTime<Utc>,
    pub source_updated_at: Option<DateTime<Utc>>,
    /// The full original, only when the search asked for `full` and the source keeps originals.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<serde_json::Map<String, serde_json::Value>>")]
    pub original: Option<serde_json::Value>,
    pub coverage: Option<Coverage>,
}

/// Matches, plus the coverage of every source searched, so an empty answer says what it covered.
#[derive(Debug, JsonSchema, Serialize)]
pub struct SearchResults {
    pub results: Vec<SearchResult>,
    /// More hits matched than `limit` allowed.
    pub truncated: bool,
    pub coverage: Vec<Coverage>,
}

/// Cuts the fetched hits (one more than `limit`) to `limit` and pairs each with its source's coverage.
pub(crate) fn assemble_search(
    mut hits: Vec<Hit>,
    coverage: Vec<Coverage>,
    options: &SearchOptions,
) -> SearchResults {
    let truncated = hits.len() > options.limit;
    hits.truncate(options.limit);
    let results = hits
        .into_iter()
        .map(|hit| {
            let e = hit.evidence;
            let covering = coverage
                .iter()
                .find(|c| c.source == e.source && c.scope == e.scope)
                .cloned();
            SearchResult {
                id: hit.id,
                matched: excerpt(&e.quote, hit.matched, e.start),
                quote: options.full.then_some(e.quote),
                original: if options.full { e.original } else { None },
                source: e.source,
                scope: e.scope,
                location: e.location,
                field: e.field,
                start: e.start,
                end: e.end,
                trust_class: e.trust_class,
                captured_at: e.captured_at,
                source_updated_at: e.source_updated_at,
                coverage: covering,
            }
        })
        .collect();
    SearchResults {
        results,
        truncated,
        coverage,
    }
}
