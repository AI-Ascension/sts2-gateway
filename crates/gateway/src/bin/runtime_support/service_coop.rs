// SPDX-License-Identifier: MIT

use super::super::coop_reports::PeerReport;
use super::*;

const SCHEMA_DIGEST: &str = "d410858cabbd38612345120c2196423130c7b21d788fd2b0d775cd82887087ec";

impl RuntimeService {
    pub(super) fn coop_synchronization(&self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if !request.body.is_empty() {
            return (400, json_error("coop_read_body_not_allowed"));
        }
        let Some(reports) = &self.coop_reports else {
            return (503, json_error("coop_roster_not_configured"));
        };
        let (generation, players, synchronization) = reports.snapshot(Instant::now());
        (
            200,
            json_bytes(&json!({
                "protocol_version": "coop-synchronization-v1",
                "schema_digest": SCHEMA_DIGEST,
                "provenance": {
                    "artifact": "sts2-protocol/coop-synchronization-v1",
                    "source": "schemas/coop-synchronization-v1.schema.json",
                    "generator": "hand-authored"
                },
                "correlation_id": request.headers.get("x-sts2-correlation-id"),
                "instance_id": self.config.instance_id,
                "session_id": self.config.session_id,
                "lease_id": self.config.lease_id,
                "lease_epoch": self.config.lease_epoch,
                "generation": generation,
                "kind": "synchronization_response",
                "source": "gateway_peer_reports",
                "players": players,
                "synchronization": synchronization,
            })),
        )
    }

    pub(super) fn coop_peer_report(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if !request.content_type_is_json() || request.body.len() > 1024 {
            return (400, json_error("coop_report_body_invalid"));
        }
        let Some(reports) = &mut self.coop_reports else {
            return (503, json_error("coop_roster_not_configured"));
        };
        let Ok(report) = serde_json::from_slice::<PeerReport>(&request.body) else {
            return (400, json_error("coop_report_body_invalid"));
        };
        match reports.report(report, Instant::now()) {
            Ok(()) => (200, json_bytes(&json!({"status": "peer_report_recorded"}))),
            Err(code) => (409, json_error(code)),
        }
    }

    pub(super) fn coop_synchronization_path(&self) -> String {
        format!(
            "/v1/instances/{}/coop/synchronization",
            self.config.instance_id
        )
    }

    pub(super) fn coop_report_path(&self) -> String {
        format!("/v1/instances/{}/coop/peer-report", self.config.instance_id)
    }
}
