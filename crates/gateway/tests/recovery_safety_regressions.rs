// SPDX-License-Identifier: MIT

//! Red recovery-store safety regressions.  The module files are split to keep
//! each test source below the repository's Rust test-file budget.

#[path = "recovery_safety_regressions/regressions.rs"]
mod regressions;
#[path = "recovery_safety_regressions/support.rs"]
mod support;
