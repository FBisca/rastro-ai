//! The read-only dispatch: the operations a server may offer without writing, run on a
//! [`Reader`].
use crate::{Request, Response, results::*};
use anyhow::{Result, bail};
use rastro_core::{ReadStorage, Reader};

impl Request {
    /// Runs the operation on a read-only view. Only the read-only operations that a reader can
    /// answer (`search`, `coverage`) run; any other is an error and changes nothing.
    pub fn run_read<S: ReadStorage>(self, reader: &mut Reader<S>) -> Result<Response> {
        Ok(match self {
            Self::Coverage(request) => Response::Coverage(CoverageResult {
                coverage: reader.coverage(request.name.as_deref(), request.all_windows)?,
            }),
            Self::Search(request) => {
                let found = reader.search(&request.text, &request.options())?;
                Response::Search(SearchResponse {
                    query: request.text,
                    results: found.results,
                    truncated: found.truncated,
                    coverage: found.coverage,
                })
            }
            other => bail!("{} can't run on a read-only view", other.name()),
        })
    }
}
