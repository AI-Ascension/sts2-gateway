// SPDX-License-Identifier: MIT

use sts2_gateway::{CallerId, CoopPeerRole, CoopSession, CoopSessionError, InstanceId, LeaseEpoch};

fn session() -> CoopSession {
    CoopSession::new(InstanceId::new(7), LeaseEpoch::new(3), 10)
}

#[test]
fn disagreement_and_disconnect_suspend_mutation() -> Result<(), String> {
    let mut session = session();
    session
        .register_peer(CallerId::new(1), CoopPeerRole::Local, 10)
        .map_err(|error| format!("{error:?}"))?;
    session
        .register_peer(CallerId::new(2), CoopPeerRole::Ally, 10)
        .map_err(|error| format!("{error:?}"))?;
    assert!(session.authorize_mutation(10).is_ok());

    session
        .update_generation(CallerId::new(2), 11)
        .map_err(|error| format!("{error:?}"))?;
    assert!(session.snapshot().disagreement());
    assert_eq!(
        session.authorize_mutation(10),
        Err(CoopSessionError::MutationSuspended)
    );

    session
        .reconnect(CallerId::new(2), 10)
        .map_err(|error| format!("{error:?}"))?;
    session
        .disconnect(CallerId::new(2))
        .map_err(|error| format!("{error:?}"))?;
    assert!(!session.snapshot().missing_peers().is_empty());
    assert_eq!(
        session.authorize_mutation(10),
        Err(CoopSessionError::MutationSuspended)
    );
    Ok(())
}

#[test]
fn local_identity_and_peer_capacity_are_explicit() -> Result<(), String> {
    let mut session = session();
    session
        .register_peer(CallerId::new(1), CoopPeerRole::Local, 10)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        session.register_peer(CallerId::new(2), CoopPeerRole::Local, 10),
        Err(CoopSessionError::DuplicateLocalPeer)
    );
    for peer in 2..=4 {
        session
            .register_peer(CallerId::new(peer), CoopPeerRole::Ally, 10)
            .map_err(|error| format!("{error:?}"))?;
    }
    assert_eq!(
        session.register_peer(CallerId::new(5), CoopPeerRole::Ally, 10),
        Err(CoopSessionError::PeerCapacity)
    );
    Ok(())
}
