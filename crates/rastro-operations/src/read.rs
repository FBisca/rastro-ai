#![allow(non_snake_case)]
//! The read-only dispatch: the operations a server may offer without writing, run on a
//! [`Reader`].
use crate::{requests::*, results::*};
use anyhow::Result;
use rastro_core::{ReadStorage, Reader};

impl CoverageRequest {
    pub(crate) fn run_read<S: ReadStorage>(self, reader: &mut Reader<S>) -> Result<CoverageResult> {
        Ok(CoverageResult {
            coverage: reader.coverage(self.name.as_deref(), self.all_windows)?,
        })
    }
}
impl SearchRequest {
    pub(crate) fn run_read<S: ReadStorage>(self, reader: &mut Reader<S>) -> Result<SearchResponse> {
        let found = reader.search(&self.text, &self.options())?;
        Ok(SearchResponse {
            query: self.text,
            results: found.results,
            truncated: found.truncated,
            coverage: found.coverage,
        })
    }
}

impl AboutRequest {
    pub(crate) fn run_read<S: ReadStorage>(self, reader: &mut Reader<S>) -> Result<AboutResult> {
        reader.about(&self.entity, &self.options())
    }
}

impl MeetingCommitmentsRequest {
    pub(crate) fn run_read<S: ReadStorage>(self, reader: &mut Reader<S>) -> Result<MeetingResult> {
        reader.meeting_commitments(&self.meeting)
    }
}

impl WhyRequest {
    pub(crate) fn run_read<S: ReadStorage>(self, reader: &mut Reader<S>) -> Result<WhyResult> {
        reader.explain(&self.claim, &self.options())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog;
    use rastro_core::{Locations, Rastro, Snapshot};
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct MissingStorage;
    impl ReadStorage for MissingStorage {
        fn read<T>(&mut self, _work: impl FnOnce(&dyn Snapshot) -> Result<T>) -> Result<T> {
            panic!("a missing database has no storage to read")
        }
    }

    /// surfaces/R2: catalog requests dispatch to supported reader operations or reject explicitly.
    #[test]
    fn read_dispatch_accepts_only_supported_catalog_operations() {
        // A missing home exercises the real Reader without files or a storage dependency.
        let home = std::env::temp_dir().join(format!(
            "rastro-read-dispatch-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert!(!home.exists());
        let mut reader = Rastro::open_read_only(
            Locations {
                data: home.join("data"),
                config: home.join("config"),
            },
            |_| Ok(None::<MissingStorage>),
        )
        .unwrap();
        let cases = [
            (
                "source.add",
                json!({"name": "a", "connector": "github", "repo": "o/r", "scope": "w"}),
                None,
            ),
            ("source.list", json!({}), None),
            ("sync", json!({}), None),
            ("index.rebuild", json!({}), None),
            (
                "search",
                json!({"text": "evidence"}),
                Some(
                    json!({"query": "evidence", "results": [], "truncated": false, "coverage": []}),
                ),
            ),
            ("coverage", json!({}), Some(json!({"coverage": []}))),
        ];
        for (name, arguments, expected) in cases {
            let request = catalog::find(name).unwrap().decode(arguments).unwrap();
            let outcome = request.run_read(&mut reader);
            match expected {
                Some(result) => {
                    let response = outcome.unwrap_or_else(|error| panic!("{name}: {error}"));
                    assert_eq!(response.operation(), name);
                    assert_eq!(response.result(), result, "{name}");
                }
                None => assert_eq!(
                    outcome.unwrap_err().to_string(),
                    format!("{name} can't run on a read-only view")
                ),
            }
        }
        assert!(!home.exists());
    }
}
