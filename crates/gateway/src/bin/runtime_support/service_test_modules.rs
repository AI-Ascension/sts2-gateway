// SPDX-License-Identifier: MIT

// Test-only module registration, included under cfg(test) by the service.

#[path = "service_admission_tests.rs"]
mod admission_tests;

#[path = "service_allocation_cleanup_tests.rs"]
mod allocation_cleanup_tests;
#[path = "service_allocation_failure_tests.rs"]
mod allocation_failure_tests;
#[path = "service_auth_tests.rs"]
mod auth_tests;
#[path = "service_tests.rs"]
mod legacy_tests;
#[path = "service_recovery_catalog_tests.rs"]
mod recovery_catalog_tests;
#[path = "service_routes_tests.rs"]
mod routes_tests;
#[path = "service_v3_catalog_tests.rs"]
mod runtime_v3_catalog_tests;
#[path = "service_support_tests.rs"]
mod test_support;

#[path = "service_coop_tests.rs"]
mod coop_tests;
