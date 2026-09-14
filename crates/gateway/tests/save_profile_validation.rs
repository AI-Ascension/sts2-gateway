// SPDX-License-Identifier: MIT

use sts2_gateway::{
    InMemoryLaunchProfileBindingPort, InMemorySaveProfileRecordStore, InMemoryUserDataPort,
    InMemoryUserDataRecordStore, LAUNCH_PROFILE_ID, SaveProfileAuthority, SaveProfileContext,
    SaveProfileForwardRequest, SaveProfileForwardingPort, SaveProfileLedger, SaveProfileOperation,
    SaveProfileRoute, SaveProfileTransportFault,
};

#[derive(Default)]
struct NoopForwarder;

impl SaveProfileForwardingPort for NoopForwarder {
    fn forward(
        &mut self,
        _request: SaveProfileForwardRequest,
    ) -> Result<sts2_gateway::SaveProfileForwardResponse, SaveProfileTransportFault> {
        Err(SaveProfileTransportFault::UnavailableBeforeWrite)
    }

    fn lookup(
        &mut self,
        _request: SaveProfileForwardRequest,
    ) -> Result<Option<sts2_gateway::SaveProfileForwardResponse>, SaveProfileTransportFault> {
        Ok(None)
    }
}

fn context() -> SaveProfileContext {
    SaveProfileContext {
        instance_id: String::from("instance-1"),
        caller_id: String::from("caller-1"),
        session_id: String::from("session-1"),
        lease_id: String::from("lease-1"),
        lease_epoch: 1,
        correlation_id: String::from("corr"),
    }
}

fn authority() -> SaveProfileAuthority {
    SaveProfileAuthority {
        instance_id: String::from("instance-1"),
        caller_id: String::from("caller-1"),
        session_id: String::from("session-1"),
        lease_id: String::from("lease-1"),
        lease_epoch: 1,
        expires_at_millis: None,
    }
}

#[test]
fn operation_ids_cannot_be_used_as_paths() -> Result<(), String> {
    let request = SaveProfileForwardRequest {
        operation_id: String::from("../escape"),
        context: context(),
        route: SaveProfileRoute::Current,
        operation: SaveProfileOperation::Current,
        body: Vec::new(),
    };
    let mut ledger = SaveProfileLedger::new(
        8,
        authority(),
        NoopForwarder,
        InMemorySaveProfileRecordStore::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        ledger.submit(request, 0, false),
        Err(sts2_gateway::SaveProfileLedgerError::InvalidRequest)
    );

    let mut provisioner = sts2_gateway::UserDataProvisioner::new(
        4,
        InMemoryUserDataPort::default(),
        InMemoryUserDataRecordStore::default(),
        InMemoryLaunchProfileBindingPort::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        provisioner.create_disposable(context(), "../escape", LAUNCH_PROFILE_ID),
        Err(sts2_gateway::UserDataProvisioningError::InvalidRequest)
    );
    Ok(())
}
