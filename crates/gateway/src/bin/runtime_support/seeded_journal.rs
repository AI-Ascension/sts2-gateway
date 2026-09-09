// SPDX-License-Identifier: MIT

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sts2_gateway::SeededRunPersistedState;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SeededJournalDocument {
    format_version: u32,
    state: SeededRunPersistedState,
}

/// Loads the sidecar seeded-run marker. It is separate from Runtime-v2 state so the existing
/// gameplay journal format and digest remain unchanged.
pub(crate) fn seeded_load(path: &Path) -> Result<Option<SeededRunPersistedState>, String> {
    let path = seeded_path(path);
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("seeded-run journal read failed: {error}")),
    };
    let mut bytes = Vec::new();
    file.take((super::MAX_JOURNAL_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("seeded-run journal read failed: {error}"))?;
    if bytes.len() > super::MAX_JOURNAL_BYTES {
        return Err(String::from("seeded-run journal exceeds its size bound"));
    }
    let document = serde_json::from_slice::<SeededJournalDocument>(&bytes)
        .map_err(|error| format!("seeded-run journal is malformed: {error}"))?;
    if document.format_version != super::JOURNAL_FORMAT_VERSION {
        return Err(format!(
            "unsupported seeded-run journal format {}",
            document.format_version
        ));
    }
    Ok(Some(document.state))
}

pub(crate) fn seeded_store(path: &Path, state: &SeededRunPersistedState) -> Result<(), String> {
    let path = seeded_path(path);
    let document = SeededJournalDocument {
        format_version: super::JOURNAL_FORMAT_VERSION,
        state: state.clone(),
    };
    let bytes = serde_json::to_vec(&document)
        .map_err(|error| format!("seeded-run journal serialization failed: {error}"))?;
    if bytes.len() > super::MAX_JOURNAL_BYTES {
        return Err(String::from("seeded-run journal exceeds its size bound"));
    }
    super::ensure_parent_directory(&path)?;
    let (temporary_path, mut file) = super::create_temporary(&path)?;
    let result = (|| {
        file.write_all(&bytes)
            .map_err(|error| format!("seeded-run journal write failed: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("seeded-run journal sync failed: {error}"))?;
        drop(file);
        fs::rename(&temporary_path, &path)
            .map_err(|error| format!("seeded-run journal replace failed: {error}"))?;
        super::sync_parent_directory(&path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

fn seeded_path(path: &Path) -> PathBuf {
    path.with_extension("seeded-run.json")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use sts2_gateway::{
        SeededRunBinding, SeededRunForwardRequest, SeededRunForwardingPort, SeededRunLedger,
        SeededRunLedgerConfig, SeededRunMessage, SeededRunPersistedOperation,
        SeededRunPersistedState, SeededRunReceiptRequest, SeededRunTransportFault,
    };

    use super::{seeded_load, seeded_store};

    const SEEDED_START_REQUEST: &str =
        include_str!("../../../../../protocol-artifact/seeded-run-v1/golden/start-request.json");
    const SEEDED_UNKNOWN: &str =
        include_str!("../../../../../protocol-artifact/seeded-run-v1/golden/start-unknown.json");
    const SEEDED_RECONCILE: &str = include_str!(
        "../../../../../protocol-artifact/seeded-run-v1/golden/reconcile-request.json"
    );
    const SEEDED_SETTLED: &str = include_str!(
        "../../../../../protocol-artifact/seeded-run-v1/golden/reconcile-settled.json"
    );

    struct SeededJournalForwarder {
        starts: usize,
        receipts: usize,
    }

    impl SeededRunForwardingPort for SeededJournalForwarder {
        fn forward_seeded_run(
            &mut self,
            _request: SeededRunForwardRequest,
        ) -> Result<SeededRunMessage, SeededRunTransportFault> {
            self.starts += 1;
            Err(SeededRunTransportFault::MalformedResponse)
        }

        fn read_seeded_run_receipt(
            &mut self,
            _request: SeededRunReceiptRequest,
        ) -> Result<Option<SeededRunMessage>, SeededRunTransportFault> {
            self.receipts += 1;
            serde_json::from_str(SEEDED_SETTLED)
                .map(Some)
                .map_err(|_| SeededRunTransportFault::MalformedResponse)
        }
    }

    fn test_path() -> PathBuf {
        let name: String = std::thread::current()
            .name()
            .unwrap_or("seeded-journal")
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        std::env::temp_dir().join(format!(
            "sts2-seeded-run-journal-{}-{name}.json",
            std::process::id()
        ))
    }

    #[test]
    fn seeded_journal_restores_unknown_and_reconciles_without_redispatch() -> Result<(), String> {
        let path = test_path();
        let mut request: SeededRunMessage =
            serde_json::from_str(SEEDED_START_REQUEST).map_err(|e| e.to_string())?;
        request.correlation_id = String::from("corr-seed-0003");
        request.operation_id = String::from("op-seed-unknown");
        request.run_mode = Some(sts2_gateway::SeededRunMode::SeededReplay);
        let unknown: SeededRunMessage =
            serde_json::from_str(SEEDED_UNKNOWN).map_err(|e| e.to_string())?;
        let state = SeededRunPersistedState {
            binding: SeededRunBinding::new("instance-1", "session-1", "lease-1", 1, 0)
                .map_err(|e| e.to_string())?,
            operations: vec![SeededRunPersistedOperation {
                request,
                result: Some(unknown),
            }],
        };
        let result = (|| {
            seeded_store(&path, &state)?;
            let restored =
                seeded_load(&path)?.ok_or_else(|| String::from("seeded journal was not loaded"))?;
            let mut ledger = SeededRunLedger::new(
                SeededRunLedgerConfig::new(8),
                SeededRunBinding::new("instance-1", "session-1", "lease-1", 1, 0)
                    .map_err(|e| e.to_string())?,
                SeededJournalForwarder {
                    starts: 0,
                    receipts: 0,
                },
            )
            .map_err(|e| e.to_string())?;
            ledger.restore_state(restored).map_err(|e| e.to_string())?;
            let reconcile: SeededRunMessage =
                serde_json::from_str(SEEDED_RECONCILE).map_err(|e| e.to_string())?;
            let settled = ledger.reconcile(reconcile).map_err(|e| e.to_string())?;
            if settled.status != Some(sts2_gateway::SeededRunStatus::Settled)
                || settled.validate().is_err()
                || ledger.forwarding_mut().starts != 0
                || ledger.forwarding_mut().receipts != 1
            {
                return Err(String::from(
                    "seeded journal reconciliation redispatched or failed to settle",
                ));
            }
            Ok(())
        })();
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("seeded-run.json"));
        result
    }
}
