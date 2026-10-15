//! Helpers shared by the engine's tests.

use std::path::Path;
use std::time::Duration;

use tokio::time::Instant;

use crate::project::{Project, ProjectFile};

/// How long a test waits for the engine to reach an expected state.
pub(super) const WAIT: Duration = Duration::from_secs(5);

/// Parses and validates `src`, which must be a valid project.
pub(super) fn valid_project(src: &str) -> Project {
    let validated = ProjectFile::parse(src).unwrap().validate(Path::new("."));
    assert!(validated.errors.is_empty(), "{:?}", validated.errors);
    validated.project
}

/// Polls `cond` until it holds; fails with `timeout_message()` once [`WAIT`] has passed.
pub(super) async fn eventually(timeout_message: impl Fn() -> String, cond: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !cond() {
        assert!(Instant::now() < deadline, "{}", timeout_message());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
