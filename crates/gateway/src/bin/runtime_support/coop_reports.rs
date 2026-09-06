// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};

const MAX_GENERATION: u64 = 9_007_199_254_740_991;
pub(super) const REPORT_LIFETIME: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Deserialize, serde::Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Role {
    Local,
    Ally,
}

#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Member {
    peer_id: String,
    role: Role,
}

struct Peer {
    member: Member,
    generation: Option<u64>,
    reported_at: Option<Instant>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PeerReport {
    peer_id: String,
    generation: u64,
    connected: bool,
}

/// A coordinator-report ledger, never a host mutation barrier or peer authenticator.
pub(super) struct CoopReports {
    peers: Vec<Peer>,
    generation: u64,
    lifetime: Duration,
}

impl CoopReports {
    pub(super) fn from_roster(text: &str) -> Result<Self, String> {
        if text.len() > 2048 {
            return Err("co-op roster exceeds its byte bound".to_owned());
        }
        let members: Vec<Member> = serde_json::from_str(text)
            .map_err(|_| "co-op roster must be a closed array of peer IDs and roles".to_owned())?;
        validate_roster(&members)?;
        Ok(Self {
            peers: members
                .into_iter()
                .map(|member| Peer {
                    member,
                    generation: None,
                    reported_at: None,
                })
                .collect(),
            generation: 0,
            lifetime: REPORT_LIFETIME,
        })
    }

    pub(super) fn report(&mut self, report: PeerReport, now: Instant) -> Result<(), &'static str> {
        if report.generation > MAX_GENERATION {
            return Err("coop_generation_out_of_bounds");
        }
        let Some(peer) = self
            .peers
            .iter_mut()
            .find(|peer| peer.member.peer_id == report.peer_id)
        else {
            return Err("coop_unknown_peer");
        };
        if report.generation < self.generation
            || peer
                .generation
                .is_some_and(|previous| report.generation < previous)
        {
            return Err("coop_generation_regression");
        }
        peer.generation = Some(report.generation);
        peer.reported_at = report.connected.then_some(now);
        self.advance(now);
        Ok(())
    }

    fn advance(&mut self, now: Instant) {
        let Some(generation) = self.peers.first().and_then(|peer| peer.generation) else {
            return;
        };
        if generation >= self.generation
            && self
                .peers
                .iter()
                .all(|peer| fresh(peer, now, self.lifetime) && peer.generation == Some(generation))
        {
            self.generation = generation;
        }
    }

    pub(super) fn snapshot(&self, now: Instant) -> (u64, Value, Value) {
        let missing: Vec<&str> = self
            .peers
            .iter()
            .filter(|peer| !fresh(peer, now, self.lifetime))
            .map(|peer| peer.member.peer_id.as_str())
            .collect();
        let status = if !missing.is_empty() {
            "disconnected"
        } else if self
            .peers
            .iter()
            .any(|peer| peer.generation != Some(self.generation))
        {
            "disagreement"
        } else {
            "synchronized"
        };
        let players: Vec<&Member> = self.peers.iter().map(|peer| &peer.member).collect();
        (
            self.generation,
            json!(players),
            json!({
                "status": status,
                "generation": self.generation,
                "peer_count": self.peers.len(),
                "missing_peers": missing,
            }),
        )
    }
}

fn fresh(peer: &Peer, now: Instant, lifetime: Duration) -> bool {
    peer.reported_at
        .and_then(|at| now.checked_duration_since(at))
        .is_some_and(|age| age < lifetime)
}

fn validate_roster(members: &[Member]) -> Result<(), String> {
    if !(2..=4).contains(&members.len()) {
        return Err("co-op roster requires two to four peers".to_owned());
    }
    let mut ids = BTreeSet::new();
    let mut locals = 0;
    for member in members {
        let id = &member.peer_id;
        if id.is_empty()
            || id.len() > 128
            || !ids.insert(id)
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
        {
            return Err("co-op peer IDs must be unique bounded identities".to_owned());
        }
        if member.role == Role::Local {
            locals += 1;
        }
    }
    if locals != 1 {
        return Err("co-op roster requires exactly one local peer".to_owned());
    }
    Ok(())
}

#[cfg(test)]
#[path = "coop_reports_tests.rs"]
mod tests;
