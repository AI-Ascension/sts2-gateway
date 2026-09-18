// SPDX-License-Identifier: MIT

//! Shared fixtures for the process-lifecycle tests.
//!
//! Kept in one place so the HTTP boundary tests and the configuration tests build the same
//! server-owned catalog and never share a durable store path, which the coordinator locks
//! exclusively for its lifetime.

/// The approved launch profiles the process-lifecycle fixtures configure.
pub(super) fn profiles(
) -> Result<Vec<super::service_process_lifecycle_config::ConfiguredLaunchProfile>, String> {
    super::service_process_lifecycle_config::parse_profiles(
        "7:11:12:13:14:4:5000:5000;9:21:22:23:24:4:5000:5000",
    )
    .map_err(|error| format!("fixture profiles are valid: {error:?}"))
}

/// A store path unique to one test, so parallel tests never share a coordinator lock.
pub(super) fn store_path() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "sts2-lifecycle-{}-{}.sqlite",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    path
}
