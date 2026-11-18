//! Used by `scripts/nextest-config.test.sh` to watch nextest report SLOW, terminate and name a test
//! that runs past the repository's 20-second period three times.

use std::thread::sleep;
use std::time::Duration;

#[test]
fn test_outlives_the_timeout() {
    // Longer than the 60 seconds the config allows, so a config that fails to terminate the test also
    // fails the check that runs it, instead of passing by accident.
    sleep(Duration::from_secs(90));
}
