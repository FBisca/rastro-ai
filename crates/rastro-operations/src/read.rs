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
