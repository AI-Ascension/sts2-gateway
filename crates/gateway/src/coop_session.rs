// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use crate::identity::{CallerId, InstanceId, LeaseEpoch};

const MAX_PEERS: usize = 4;
const MAX_GENERATION: u64 = 9_007_199_254_740_991;

/// Role assigned by the gateway session owner; it is not a gameplay action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoopPeerRole {
    Local,
    Ally,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PeerStatus {
    Connected,
    Disconnected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PeerBinding {
    role: CoopPeerRole,
    generation: u64,
    status: PeerStatus,
}

/// Synchronization status exported to the harness/MCP coordination lane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoopSynchronizationSnapshot {
    instance_id: InstanceId,
    lease_epoch: LeaseEpoch,
    generation: u64,
    peer_count: usize,
    missing_peers: Vec<CallerId>,
    disagreement: bool,
}

impl CoopSynchronizationSnapshot {
    #[must_use]
    pub const fn instance_id(&self) -> InstanceId {
        self.instance_id
    }
    #[must_use]
    pub const fn lease_epoch(&self) -> LeaseEpoch {
        self.lease_epoch
    }
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    #[must_use]
    pub const fn peer_count(&self) -> usize {
        self.peer_count
    }
    #[must_use]
    pub fn missing_peers(&self) -> &[CallerId] {
        &self.missing_peers
    }
    #[must_use]
    pub const fn disagreement(&self) -> bool {
        self.disagreement
    }
    #[must_use]
    pub const fn mutation_allowed(&self) -> bool {
        !self.disagreement && self.missing_peers.is_empty() && self.peer_count >= 2
    }
}

/// Bounded gateway-owned peer synchronization ledger. It never interprets game state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoopSession {
    instance_id: InstanceId,
    lease_epoch: LeaseEpoch,
    generation: u64,
    peers: BTreeMap<CallerId, PeerBinding>,
}

impl CoopSession {
    pub const fn new(instance_id: InstanceId, lease_epoch: LeaseEpoch, generation: u64) -> Self {
        Self {
            instance_id,
            lease_epoch,
            generation,
            peers: BTreeMap::new(),
        }
    }

    pub fn register_peer(
        &mut self,
        peer_id: CallerId,
        role: CoopPeerRole,
        generation: u64,
    ) -> Result<(), CoopSessionError> {
        if self.peers.len() >= MAX_PEERS && !self.peers.contains_key(&peer_id) {
            return Err(CoopSessionError::PeerCapacity);
        }
        if generation > MAX_GENERATION {
            return Err(CoopSessionError::GenerationOutOfBounds);
        }
        if self.peers.contains_key(&peer_id) {
            return Err(CoopSessionError::DuplicatePeer);
        }
        if generation != self.generation {
            return Err(CoopSessionError::GenerationDisagreement);
        }
        if role == CoopPeerRole::Local
            && self
                .peers
                .values()
                .any(|peer| peer.role == CoopPeerRole::Local)
        {
            return Err(CoopSessionError::DuplicateLocalPeer);
        }
        self.peers.insert(
            peer_id,
            PeerBinding {
                role,
                generation,
                status: PeerStatus::Connected,
            },
        );
        Ok(())
    }

    pub fn update_generation(
        &mut self,
        peer_id: CallerId,
        generation: u64,
    ) -> Result<(), CoopSessionError> {
        if generation > MAX_GENERATION {
            return Err(CoopSessionError::GenerationOutOfBounds);
        }
        let peer = self
            .peers
            .get_mut(&peer_id)
            .ok_or(CoopSessionError::UnknownPeer)?;
        peer.generation = generation;
        Ok(())
    }

    pub fn disconnect(&mut self, peer_id: CallerId) -> Result<(), CoopSessionError> {
        let peer = self
            .peers
            .get_mut(&peer_id)
            .ok_or(CoopSessionError::UnknownPeer)?;
        peer.status = PeerStatus::Disconnected;
        Ok(())
    }

    pub fn reconnect(
        &mut self,
        peer_id: CallerId,
        generation: u64,
    ) -> Result<(), CoopSessionError> {
        if generation > MAX_GENERATION {
            return Err(CoopSessionError::GenerationOutOfBounds);
        }
        let peer = self
            .peers
            .get_mut(&peer_id)
            .ok_or(CoopSessionError::UnknownPeer)?;
        peer.generation = generation;
        peer.status = PeerStatus::Connected;
        Ok(())
    }

    pub fn authorize_mutation(&self, generation: u64) -> Result<(), CoopSessionError> {
        let snapshot = self.snapshot();
        if generation != snapshot.generation {
            return Err(CoopSessionError::GenerationDisagreement);
        }
        if !self
            .peers
            .values()
            .any(|peer| peer.role == CoopPeerRole::Local)
        {
            return Err(CoopSessionError::MissingLocalPeer);
        }
        if !snapshot.mutation_allowed() {
            return Err(CoopSessionError::MutationSuspended);
        }
        Ok(())
    }

    #[must_use]
    pub fn snapshot(&self) -> CoopSynchronizationSnapshot {
        let mut missing_peers = Vec::new();
        let mut disagreement = false;
        for (peer_id, peer) in &self.peers {
            if peer.status == PeerStatus::Disconnected {
                missing_peers.push(*peer_id);
            }
            if peer.generation != self.generation {
                disagreement = true;
            }
        }
        CoopSynchronizationSnapshot {
            instance_id: self.instance_id,
            lease_epoch: self.lease_epoch,
            generation: self.generation,
            peer_count: self.peers.len(),
            missing_peers,
            disagreement,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoopSessionError {
    PeerCapacity,
    DuplicatePeer,
    DuplicateLocalPeer,
    UnknownPeer,
    GenerationOutOfBounds,
    MissingLocalPeer,
    GenerationDisagreement,
    MutationSuspended,
}

impl std::fmt::Display for CoopSessionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::PeerCapacity => "co-op peer capacity is exhausted",
            Self::DuplicatePeer => "co-op peer is already registered",
            Self::DuplicateLocalPeer => "co-op session already has a local peer",
            Self::UnknownPeer => "co-op peer is unknown",
            Self::GenerationOutOfBounds => "co-op generation exceeds its safe integer bound",
            Self::MissingLocalPeer => "co-op session has no local peer",
            Self::GenerationDisagreement => "co-op peers disagree on generation",
            Self::MutationSuspended => "co-op mutation is suspended until synchronization recovers",
        })
    }
}

impl std::error::Error for CoopSessionError {}
