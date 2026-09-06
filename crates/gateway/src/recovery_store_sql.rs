// SPDX-License-Identifier: MIT

pub(super) const CREATE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS authority (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    deployment_id TEXT NOT NULL,
    instance_id TEXT NOT NULL,
    instance_incarnation TEXT NOT NULL,
    boot_id TEXT NOT NULL,
    authority_generation INTEGER NOT NULL,
    release_digest TEXT NOT NULL,
    config_digest TEXT NOT NULL,
    profile_digest TEXT NOT NULL,
    runtime_v3_schema_digest TEXT NOT NULL,
    created_at_millis INTEGER NOT NULL,
    state TEXT NOT NULL,
    host_fence_id TEXT,
    fence_generation INTEGER,
    host_fence_at INTEGER
);
CREATE TABLE IF NOT EXISTS leases (
    lease_id TEXT PRIMARY KEY,
    deployment_id TEXT NOT NULL,
    instance_id TEXT NOT NULL,
    instance_incarnation TEXT NOT NULL,
    boot_id TEXT NOT NULL,
    authority_generation INTEGER NOT NULL,
    lease_epoch INTEGER NOT NULL,
    fence_token_hash TEXT NOT NULL,
    issued_at_millis INTEGER NOT NULL,
    expires_at_millis INTEGER NOT NULL,
    ttl_seconds INTEGER NOT NULL,
    renewal_interval_seconds INTEGER NOT NULL,
    last_renew_sequence INTEGER NOT NULL,
    caller_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    status TEXT NOT NULL,
    revoked_reason TEXT
);
CREATE INDEX IF NOT EXISTS leases_status_idx ON leases(status);
CREATE TABLE IF NOT EXISTS operations (
    instance_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    deployment_id TEXT NOT NULL,
    instance_incarnation TEXT NOT NULL,
    boot_id TEXT NOT NULL,
    authority_generation INTEGER NOT NULL,
    lease_id TEXT NOT NULL,
    lease_epoch INTEGER NOT NULL,
    state TEXT NOT NULL,
    payload_digest TEXT NOT NULL,
    schema_digest TEXT NOT NULL,
    canonical_json BLOB NOT NULL,
    expected_state_id TEXT NOT NULL,
    expected_generation INTEGER NOT NULL,
    catalog_digest TEXT NOT NULL,
    response_status INTEGER,
    response_body BLOB,
    witness_json BLOB,
    uncertainty_reason TEXT,
    created_at_millis INTEGER NOT NULL,
    updated_at_millis INTEGER NOT NULL,
    PRIMARY KEY (instance_id, operation_id)
);
CREATE INDEX IF NOT EXISTS operations_state_idx ON operations(state);
CREATE TABLE IF NOT EXISTS admission_tickets (
    ticket_id TEXT PRIMARY KEY,
    operation_id TEXT NOT NULL,
    instance_id TEXT NOT NULL,
    payload_digest TEXT NOT NULL,
    boot_id TEXT NOT NULL,
    instance_incarnation TEXT NOT NULL,
    lease_epoch INTEGER NOT NULL,
    host_fence_id TEXT NOT NULL,
    state TEXT NOT NULL,
    issued_at_millis INTEGER NOT NULL,
    expires_at_millis INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS admission_tickets_operation_idx
    ON admission_tickets(instance_id, operation_id);
CREATE TABLE IF NOT EXISTS operation_archive (
    instance_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    deployment_id TEXT NOT NULL,
    instance_incarnation TEXT NOT NULL,
    boot_id TEXT NOT NULL,
    authority_generation INTEGER NOT NULL,
    lease_id TEXT NOT NULL,
    lease_epoch INTEGER NOT NULL,
    state TEXT NOT NULL,
    payload_digest TEXT NOT NULL,
    schema_digest TEXT NOT NULL,
    canonical_json BLOB NOT NULL,
    expected_state_id TEXT NOT NULL,
    expected_generation INTEGER NOT NULL,
    catalog_digest TEXT NOT NULL,
    response_status INTEGER,
    response_body BLOB,
    witness_json BLOB,
    uncertainty_reason TEXT,
    created_at_millis INTEGER NOT NULL,
    updated_at_millis INTEGER NOT NULL,
    archived_at_millis INTEGER NOT NULL,
    PRIMARY KEY (instance_id, operation_id)
);
CREATE INDEX IF NOT EXISTS operation_archive_digest_idx
    ON operation_archive(instance_id, operation_id, payload_digest);
PRAGMA user_version = 2;
"#;

pub(super) const OPERATION_COLUMNS: &str =
    "instance_id, operation_id, deployment_id, instance_incarnation, boot_id,
     authority_generation, lease_id, lease_epoch, state, payload_digest, schema_digest,
     canonical_json, expected_state_id, expected_generation, catalog_digest,
     response_status, response_body, witness_json, uncertainty_reason,
     created_at_millis, updated_at_millis";

pub(super) const ARCHIVE_COLUMNS: &str =
    "instance_id, operation_id, deployment_id, instance_incarnation, boot_id,
     authority_generation, lease_id, lease_epoch, state, payload_digest, schema_digest,
     canonical_json, expected_state_id, expected_generation, catalog_digest,
     response_status, response_body, witness_json, uncertainty_reason,
     created_at_millis, updated_at_millis";
