// SPDX-License-Identifier: MIT

use std::collections::VecDeque;

use sts2_gateway::{
    InMemorySaveProfileRecordStore, InMemoryUserDataPort, InMemoryUserDataRecordStore,
    LAUNCH_PROFILE_ID, LaunchProfileBinding, ProfileBaseline, SaveProfileAuthority,
    SaveProfileContext, SaveProfileForwardRequest, SaveProfileForwardResponse,
    SaveProfileForwardingPort, SaveProfileId, SaveProfileLedger, SaveProfileOperation,
    SaveProfileRoute, SaveProfileStatus, SaveProfileTransportFault, UserDataDescriptor,
    UserDataIdentity, UserDataInspection, UserDataProvenance, UserDataProvisioner,
    UserDataProvisioningStatus,
};

const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn context(correlation: &str) -> SaveProfileContext {
    SaveProfileContext {
        instance_id: String::from("instance-1"),
        caller_id: String::from("caller-1"),
        session_id: String::from("session-1"),
        lease_id: String::from("lease-1"),
        lease_epoch: 1,
        correlation_id: correlation.to_owned(),
    }
}

fn authority() -> SaveProfileAuthority {
    SaveProfileAuthority {
        instance_id: String::from("instance-1"),
        caller_id: String::from("caller-1"),
        session_id: String::from("session-1"),
        lease_id: String::from("lease-1"),
        lease_epoch: 1,
        expires_at_millis: Some(100),
    }
}

fn baseline(name: &str) -> ProfileBaseline {
    ProfileBaseline::try_new(name, DIGEST).unwrap_or_else(|_| ProfileBaseline {
        identity: String::from("fallback"),
        digest: String::from(DIGEST),
    })
}

fn descriptor(identity: u64, operation_id: &str) -> UserDataDescriptor {
    UserDataDescriptor {
        identity: UserDataIdentity::new(identity),
        provenance: UserDataProvenance {
            owner: String::from("gateway"),
            instance_id: String::from("instance-1"),
            operation_id: operation_id.to_owned(),
            contract: String::from(sts2_gateway::LAUNCH_PROFILE_CONTRACT),
        },
        baseline: None,
    }
}

#[test]
fn unknown_existing_contents_traversal_symlink_and_overwrite_are_refused() -> Result<(), String> {
    for inspection in [
        UserDataInspection::UnknownContents,
        UserDataInspection::Traversal,
        UserDataInspection::SymlinkEscape,
        UserDataInspection::Owned(UserDataProvenance {
            owner: String::from("other"),
            instance_id: String::from("other-instance"),
            operation_id: String::from("other-op"),
            contract: String::from(sts2_gateway::LAUNCH_PROFILE_CONTRACT),
        }),
    ] {
        let mut port = InMemoryUserDataPort::default();
        port.set_inspection(UserDataIdentity::new(1), inspection);
        let mut provisioner =
            UserDataProvisioner::new(4, port, InMemoryUserDataRecordStore::default())
                .map_err(|error| format!("{error:?}"))?;
        let result = provisioner
            .create_disposable(context("corr"), "op-1", LAUNCH_PROFILE_ID)
            .map_err(|error| format!("{error:?}"))?;
        assert_eq!(result.status, UserDataProvisioningStatus::Blocked);
        assert!(result.descriptor.is_none());
        assert!(provisioner.port().created.is_empty());
    }
    Ok(())
}

#[test]
fn fresh_allocations_are_unique_and_portable_descriptors_have_no_paths() -> Result<(), String> {
    let mut provisioner = UserDataProvisioner::new(
        4,
        InMemoryUserDataPort::default(),
        InMemoryUserDataRecordStore::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let first = provisioner
        .create_disposable(context("corr-1"), "op-1", LAUNCH_PROFILE_ID)
        .map_err(|error| format!("{error:?}"))?
        .descriptor
        .ok_or_else(|| String::from("first descriptor missing"))?;
    let second = provisioner
        .create_disposable(context("corr-2"), "op-2", LAUNCH_PROFILE_ID)
        .map_err(|error| format!("{error:?}"))?
        .descriptor
        .ok_or_else(|| String::from("second descriptor missing"))?;
    assert_ne!(first.identity, second.identity);
    let encoded = serde_json::to_string(&first).map_err(|error| error.to_string())?;
    assert!(!encoded.contains('/') && !encoded.contains('\\'));
    assert!(encoded.contains("\"identity\""));
    Ok(())
}

#[test]
fn timeout_reconciles_the_same_identity_without_a_second_create() -> Result<(), String> {
    let mut port = InMemoryUserDataPort::default();
    port.set_outcome(
        UserDataIdentity::new(1),
        sts2_gateway::UserDataCreateOutcome::TimeoutAfterWrite,
    );
    let mut provisioner = UserDataProvisioner::new(4, port, InMemoryUserDataRecordStore::default())
        .map_err(|error| format!("{error:?}"))?;
    let first = provisioner
        .create_disposable(context("corr"), "op-1", LAUNCH_PROFILE_ID)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(first.status, UserDataProvisioningStatus::Unknown);
    let record = provisioner
        .operation("instance-1", "op-1")
        .ok_or_else(|| String::from("record missing"))?
        .clone();
    provisioner.port_mut().set_inspection(
        record.descriptor.identity,
        UserDataInspection::Owned(record.descriptor.provenance.clone()),
    );
    let reconciled = provisioner
        .reconcile(&context("corr"), "op-1")
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(reconciled.status, UserDataProvisioningStatus::Created);
    assert_eq!(provisioner.port().created.len(), 1);
    let duplicate = provisioner
        .create_disposable(context("corr"), "op-1", LAUNCH_PROFILE_ID)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(duplicate.status, UserDataProvisioningStatus::Created);
    assert_eq!(provisioner.port().created.len(), 1);
    Ok(())
}

#[derive(Default)]
struct FakeForwarder {
    responses: VecDeque<Result<SaveProfileForwardResponse, SaveProfileTransportFault>>,
    lookups: VecDeque<Option<SaveProfileForwardResponse>>,
    forwards: usize,
    lookups_called: usize,
}

impl SaveProfileForwardingPort for FakeForwarder {
    fn forward(
        &mut self,
        _request: SaveProfileForwardRequest,
    ) -> Result<SaveProfileForwardResponse, SaveProfileTransportFault> {
        self.forwards += 1;
        self.responses
            .pop_front()
            .unwrap_or(Err(SaveProfileTransportFault::UnavailableBeforeWrite))
    }

    fn lookup(
        &mut self,
        _request: SaveProfileForwardRequest,
    ) -> Result<Option<SaveProfileForwardResponse>, SaveProfileTransportFault> {
        self.lookups_called += 1;
        Ok(self.lookups.pop_front().unwrap_or(None))
    }
}

fn read_request(
    route: SaveProfileRoute,
    operation_id: &str,
    correlation: &str,
) -> Result<SaveProfileForwardRequest, String> {
    let operation = match route {
        SaveProfileRoute::List => SaveProfileOperation::List,
        SaveProfileRoute::Current => SaveProfileOperation::Current,
        SaveProfileRoute::Select => SaveProfileOperation::Select {
            profile_id: SaveProfileId::try_new("slot-1").map_err(|_| String::from("id"))?,
            baseline: baseline("baseline-1"),
        },
        SaveProfileRoute::CreateDisposable => {
            let user_data = descriptor(1, operation_id);
            SaveProfileOperation::CreateDisposable {
                launch_profile: LaunchProfileBinding::try_new(
                    LAUNCH_PROFILE_ID,
                    user_data.identity,
                )
                .map_err(|_| String::from("binding"))?,
                user_data,
            }
        }
        SaveProfileRoute::Lookup => SaveProfileOperation::List,
    };
    Ok(SaveProfileForwardRequest {
        operation_id: operation_id.to_owned(),
        context: context(correlation),
        route,
        operation,
        body: if route.requires_body() {
            br#"{"profile_id":"slot-1"}"#.to_vec()
        } else {
            Vec::new()
        },
    })
}

fn settled(
    request: &SaveProfileForwardRequest,
    profile_id: Option<SaveProfileId>,
    baseline: Option<ProfileBaseline>,
    user_data: Option<UserDataDescriptor>,
) -> SaveProfileForwardResponse {
    SaveProfileForwardResponse {
        operation_id: request.operation_id.clone(),
        context: request.context.clone(),
        route: request.route,
        status: SaveProfileStatus::Settled,
        body: br#"{"status":"settled"}"#.to_vec(),
        profile_id,
        baseline,
        user_data,
        reason: None,
    }
}

#[test]
fn discovery_does_not_require_a_future_baseline_and_selection_is_fenced() -> Result<(), String> {
    let list = read_request(SaveProfileRoute::List, "list-1", "corr-list")?;
    let mut fake = FakeForwarder::default();
    fake.responses
        .push_back(Ok(settled(&list, None, None, None)));
    let mut ledger = SaveProfileLedger::new(
        8,
        authority(),
        fake,
        InMemorySaveProfileRecordStore::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let listed = ledger
        .submit(list, 0, true)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(listed.status, SaveProfileStatus::Settled);
    assert_eq!(ledger.forwarding_mut().forwards, 1);

    let mut selected = read_request(SaveProfileRoute::Select, "select-1", "corr-select")?;
    selected.body = br#"{"profile_id":"slot-1","baseline":{"identity":"baseline-1","digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}"#.to_vec();
    let mut fake = FakeForwarder::default();
    fake.responses.push_back(Ok(settled(
        &selected,
        Some(SaveProfileId::try_new("slot-1").map_err(|_| String::from("id"))?),
        Some(baseline("baseline-2")),
        None,
    )));
    let mut ledger = SaveProfileLedger::new(
        8,
        authority(),
        fake,
        InMemorySaveProfileRecordStore::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let result = ledger.submit(selected.clone(), 0, false);
    assert!(result.is_ok());
    let stale = SaveProfileContext {
        lease_epoch: 0,
        ..selected.context.clone()
    };
    selected.context = stale;
    assert!(matches!(
        ledger.submit(selected, 0, false),
        Err(sts2_gateway::SaveProfileLedgerError::Fence(_))
    ));
    Ok(())
}

#[test]
fn duplicate_selection_disconnect_and_unknown_retain_recovery_state() -> Result<(), String> {
    let request = read_request(SaveProfileRoute::Select, "select-1", "corr-select")?;
    let mut fake = FakeForwarder::default();
    fake.responses
        .push_back(Err(SaveProfileTransportFault::TimeoutAfterWrite));
    let mut ledger = SaveProfileLedger::new(
        8,
        authority(),
        fake,
        InMemorySaveProfileRecordStore::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let unknown = ledger
        .submit(request.clone(), 0, false)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(unknown.status, SaveProfileStatus::Unknown);
    assert!(unknown.guidance.is_some());
    assert_eq!(
        ledger.submit(request.clone(), 0, false),
        Ok(unknown.clone())
    );
    let mut rebound = request.clone();
    rebound.context.correlation_id = String::from("corr-retry");
    assert_eq!(ledger.submit(rebound, 0, false), Ok(unknown.clone()));
    assert_eq!(
        ledger
            .caller_disconnected(context("corr-select"), "select-1", 0)
            .map_err(|error| format!("{error:?}"))?,
        unknown
    );
    Ok(())
}

#[test]
fn fixed_routes_and_creation_return_authoritative_identity_and_baseline() -> Result<(), String> {
    assert_eq!(
        SaveProfileRoute::List.downstream_path(),
        "/api/v1/save-profiles"
    );
    assert_eq!(
        SaveProfileRoute::Current.downstream_path(),
        "/api/v1/save-profile/current"
    );
    assert_eq!(
        SaveProfileRoute::Select.downstream_path(),
        "/api/v1/save-profile/select"
    );
    assert_eq!(
        SaveProfileRoute::CreateDisposable.downstream_path(),
        "/api/v1/save-profile/create-disposable"
    );
    assert_eq!(
        SaveProfileRoute::Lookup.downstream_path(),
        "/api/v1/save-profile/operations"
    );

    let request = read_request(
        SaveProfileRoute::CreateDisposable,
        "create-1",
        "corr-create",
    )?;
    let user_data = descriptor(9, "create-1");
    let mut fake = FakeForwarder::default();
    fake.responses.push_back(Ok(settled(
        &request,
        None,
        Some(baseline("authoritative-baseline")),
        Some(user_data.clone()),
    )));
    let mut ledger = SaveProfileLedger::new(
        8,
        authority(),
        fake,
        InMemorySaveProfileRecordStore::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let result = ledger
        .submit(request, 0, false)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(result.status, SaveProfileStatus::Settled);
    assert_eq!(result.user_data, Some(user_data));
    assert_eq!(result.baseline, Some(baseline("authoritative-baseline")));
    Ok(())
}

#[test]
fn wrong_instance_and_malformed_receipt_fail_closed_without_replay() -> Result<(), String> {
    let request = read_request(SaveProfileRoute::Current, "current-1", "corr-current")?;
    let mut fake = FakeForwarder::default();
    let mut malformed = settled(&request, None, None, None);
    malformed.operation_id = String::from("other-operation");
    fake.responses.push_back(Ok(malformed));
    let mut ledger = SaveProfileLedger::new(
        8,
        authority(),
        fake,
        InMemorySaveProfileRecordStore::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let mut foreign = request.clone();
    foreign.context.instance_id = String::from("other-instance");
    assert!(matches!(
        ledger.submit(foreign, 0, false),
        Err(sts2_gateway::SaveProfileLedgerError::Fence(
            sts2_gateway::SaveProfileFenceError::WrongInstance
        ))
    ));
    assert!(matches!(
        ledger.submit(request.clone(), 0, false),
        Err(sts2_gateway::SaveProfileLedgerError::ResponseInvalid)
    ));
    let retained = ledger
        .submit(request, 0, false)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(retained.status, SaveProfileStatus::Unknown);
    assert_eq!(ledger.forwarding_mut().forwards, 1);
    Ok(())
}
