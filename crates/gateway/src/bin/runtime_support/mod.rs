// SPDX-License-Identifier: MIT

mod auth;
mod coop_native;
mod coop_native_forwarder;
mod coop_reports;
#[cfg(test)]
mod dependency_semantics_tests;
mod forwarder;
mod host_lease_control;
mod host_lease_control_crypto;
mod host_lease_control_frames;
mod host_lease_control_grant;
mod http;
mod journal;
mod metrics;
mod recovery_control;
mod recovery_frame;
mod runtime_map;
mod runtime_map_forwarder;
mod runtime_v3_gameplay;
mod runtime_v3_gameplay_forwarder;
mod runtime_v3_relations;
mod runtime_v4_expert;
mod runtime_v4_expert_forwarder;
mod runtime_v4_expert_rest_action;
mod runtime_v4_expert_rest_action_forwarder;
mod runtime_v4_expert_rest_action_semantics;
mod seeded_run_forwarder;
mod service;
mod strict_json;

pub(crate) use service::RuntimeService;
