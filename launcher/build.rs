//! Ensure the launcher is recompiled when the stamped source ref changes, so
//! `option_env!("MB64_SOURCE_REF")` (read in `core::bootstrap`) is re-evaluated
//! across releases even when `target/` is cached in CI.
fn main() {
    println!("cargo:rerun-if-env-changed=MB64_SOURCE_REF");
}
