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

#[test]
fn allies_without_local_never_authorize_or_advance() -> Result<(), CoopSessionError> {
    let mut session = session();
    for peer in 1..=2 {
        session.register_peer(CallerId::new(peer), CoopPeerRole::Ally, 10)?;
    }
    assert!(!session.snapshot().mutation_allowed());
    assert_eq!(
        session.authorize_mutation(10),
        Err(CoopSessionError::MissingLocalPeer)
    );
    for peer in 1..=2 {
        session.update_generation(CallerId::new(peer), 11)?;
    }
    assert_eq!(session.snapshot().generation(), 10);
    assert!(!session.snapshot().mutation_allowed());
    Ok(())
}

#[test]
fn every_peer_must_converge_before_generation_advances() -> Result<(), CoopSessionError> {
    let mut session = session();
    session.register_peer(CallerId::new(1), CoopPeerRole::Local, 10)?;
    for peer in 2..=4 {
        session.register_peer(CallerId::new(peer), CoopPeerRole::Ally, 10)?;
    }
    for peer in 1..=3 {
        session.update_generation(CallerId::new(peer), 11)?;
        assert_eq!(session.snapshot().generation(), 10);
        assert!(session.snapshot().disagreement());
        assert!(!session.snapshot().mutation_allowed());
    }
    session.update_generation(CallerId::new(4), 11)?;
    assert_eq!(session.snapshot().generation(), 11);
    assert!(!session.snapshot().disagreement());
    assert!(session.snapshot().mutation_allowed());
    assert_eq!(session.authorize_mutation(11), Ok(()));
    assert_eq!(
        session.authorize_mutation(10),
        Err(CoopSessionError::GenerationDisagreement)
    );
    Ok(())
}

#[test]
fn disconnected_peer_blocks_advancement_until_reconnected() -> Result<(), CoopSessionError> {
    let mut session = session();
    session.register_peer(CallerId::new(1), CoopPeerRole::Local, 10)?;
    session.register_peer(CallerId::new(2), CoopPeerRole::Ally, 10)?;
    session.disconnect(CallerId::new(2))?;
    session.update_generation(CallerId::new(1), 11)?;
    session.update_generation(CallerId::new(2), 11)?;
    assert_eq!(session.snapshot().generation(), 10);
    assert_eq!(session.snapshot().missing_peers(), &[CallerId::new(2)]);
    assert!(!session.snapshot().mutation_allowed());
    session.reconnect(CallerId::new(2), 11)?;
    assert_eq!(session.snapshot().generation(), 11);
    assert!(session.snapshot().mutation_allowed());
    assert_eq!(session.authorize_mutation(11), Ok(()));
    Ok(())
}

#[test]
fn lone_local_and_rollback_cannot_advance_or_lower_baseline() -> Result<(), CoopSessionError> {
    let mut session = session();
    session.register_peer(CallerId::new(1), CoopPeerRole::Local, 10)?;
    session.update_generation(CallerId::new(1), 11)?;
    assert_eq!(session.snapshot().generation(), 10);
    assert!(!session.snapshot().mutation_allowed());
    session.register_peer(CallerId::new(2), CoopPeerRole::Ally, 10)?;
    session.update_generation(CallerId::new(2), 11)?;
    assert_eq!(session.snapshot().generation(), 11);
    for peer in 1..=2 {
        session.reconnect(CallerId::new(peer), 10)?;
        assert_eq!(session.snapshot().generation(), 11);
        assert!(!session.snapshot().mutation_allowed());
    }
    assert!(session.snapshot().disagreement());
    for peer in 1..=2 {
        session.update_generation(CallerId::new(peer), 11)?;
    }
    assert_eq!(session.authorize_mutation(11), Ok(()));
    Ok(())
}
