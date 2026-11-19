//! `rastro tick`: what a schedule calls to keep every source fresh.
use anyhow::Result;
use rastro_core::{
    Locations, Rastro, display_safe,
    tick::{Tick, TickReport},
};
use rastro_storage_sqlite::SqliteStorage;

/// What the tick tells the owner, and the exit code: 0 when every unfinished run was finished and
/// every source that needed a sync got one, or when another tick was already running; 1 naming
/// each run and each source that failed.
fn outcome(tick: &Tick) -> (String, Vec<String>, i32) {
    match tick {
        Tick::AlreadyRunning => ("A tick is already running; nothing to do".into(), vec![], 0),
        Tick::Ran(TickReport {
            runs_finished,
            runs_failed,
            synced,
            fresh,
            failed,
        }) => {
            let said = format!(
                "Finished {} action run(s), {} failed; synced {} source(s), {} already fresh, {} failed",
                runs_finished.len(),
                runs_failed.len(),
                synced.len(),
                fresh.len(),
                failed.len()
            );
            let runs = runs_failed.iter().map(|(run, why)| {
                format!(
                    "action run {} failed: {}",
                    display_safe(run),
                    display_safe(why)
                )
            });
            let sources = failed.iter().map(|(source, why)| {
                format!(
                    "source {} failed to sync: {}",
                    display_safe(source),
                    display_safe(why)
                )
            });
            let errors: Vec<String> = runs.chain(sources).collect();
            (said, errors.clone(), i32::from(!errors.is_empty()))
        }
    }
}

fn tick() -> Result<Tick> {
    let locations = Locations::resolve()?;
    let storage = SqliteStorage::open(&locations.database())?;
    Rastro::open(locations, storage)?.tick()
}

/// Runs one tick and returns the process's exit code.
pub fn run() -> i32 {
    match tick() {
        Ok(tick) => {
            let (said, errors, code) = outcome(&tick);
            println!("{said}");
            for error in errors {
                eprintln!("error: {error}");
            }
            code
        }
        Err(error) => {
            eprintln!("error: {error:#}");
            1
        }
    }
}
