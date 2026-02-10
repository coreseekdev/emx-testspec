//! Integration test: run testscript files via cargo test
//!
//! This test discovers and runs all `.txtar` files in the workspace's
//! `tests/testscript/` directory.
//!
//! Usage:
//!   cargo test --package emx-testspec --test integration    # run all
//!   TESTSCRIPT_VERBOSE=1 cargo test --package emx-testspec --test integration  # verbose
//!
//! Environment variables:
//!   TESTSCRIPT_VERBOSE=1  — print script execution log
//!   TESTSCRIPT_WORK=1     — preserve working directories

use std::path::PathBuf;

/// Find the workspace root by looking for the top-level Cargo.toml
fn workspace_root() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    // emx-testspec is at crates/emx-testspec, workspace root is 2 levels up
    PathBuf::from(manifest).join("../../").canonicalize().unwrap()
}

#[test]
fn testscript_all() {
    let dir = workspace_root().join("tests/testscript");

    if !dir.exists() {
        eprintln!("No testscript directory at: {}", dir.display());
        return;
    }

    emx_testspec::run_and_assert(dir);
}
