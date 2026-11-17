//! Used by `scripts/nextest-config.test.sh`: both failing tests must reach the JUnit report with
//! their output, and must be the only tests a rerun of the latest run executes.

#[test]
fn test_that_passes() {
    assert_eq!(2 + 2, 4);
}

#[test]
fn test_that_fails() {
    println!("fixture: captured output before failing");
    panic!("fixture: intended failure");
}

#[test]
fn test_that_also_fails() {
    println!("fixture: second failure output");
    panic!("fixture: second intended failure");
}
