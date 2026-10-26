// SPDX-License-Identifier: MIT

use super::*;

fn rotated_disconnect_context(field: &str) -> (SaveProfileAuthority, SaveProfileContext) {
    let mut rotated = authority();
    let mut current = context("corr-disconnect");
    match field {
        "caller" => {
            rotated.caller_id = String::from("caller-2");
            current.caller_id = rotated.caller_id.clone();
        }
        "session" => {
            rotated.session_id = String::from("session-2");
            current.session_id = rotated.session_id.clone();
        }
        "lease" => {
            rotated.lease_id = String::from("lease-2");
            current.lease_id = rotated.lease_id.clone();
        }
        _ => {
            rotated.lease_epoch = 2;
            current.lease_epoch = rotated.lease_epoch;
        }
    }
    (rotated, current)
}

#[test]
fn disconnect_cannot_access_an_operation_after_authority_rotation() -> Result<(), String> {
    for field in ["caller", "session", "lease", "epoch"] {
        for terminal in [false, true] {
            let request = read_request(SaveProfileRoute::List, "list-1", "corr-list")?;
            let mut response = settled(&request, None, None, None);
            if !terminal {
                response.status = SaveProfileStatus::Accepted;
            }
            let mut fake = FakeForwarder::default();
            fake.responses.push_back(Ok(response));
            let mut ledger = SaveProfileLedger::new(
                8,
                authority(),
                fake,
                InMemorySaveProfileRecordStore::default(),
            )
            .map_err(|error| format!("{error:?}"))?;
            ledger
                .submit(request, 0, false)
                .map_err(|error| format!("{error:?}"))?;
            let original = ledger
                .operation("instance-1", "list-1")
                .ok_or("missing operation")?
                .clone();
            let (rotated, current) = rotated_disconnect_context(field);
            ledger.set_authority(rotated);
            assert_eq!(
                ledger.caller_disconnected(current, "list-1", 0),
                Err(sts2_gateway::SaveProfileLedgerError::Fence(
                    sts2_gateway::SaveProfileFenceError::WrongSession
                )),
                "{field}, terminal={terminal}"
            );
            assert_eq!(ledger.operation("instance-1", "list-1"), Some(&original));
            assert_eq!(ledger.forwarding_mut().forwards, 1);
            assert_eq!(ledger.forwarding_mut().lookups_called, 0);
            ledger.set_authority(authority());
            let result = ledger
                .caller_disconnected(context("new-correlation"), "list-1", 0)
                .map_err(|error| format!("{error:?}"))?;
            assert_eq!(
                result.status,
                if terminal {
                    SaveProfileStatus::Settled
                } else {
                    SaveProfileStatus::Unknown
                }
            );
        }
    }
    Ok(())
}
