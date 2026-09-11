// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::collections::BTreeSet;

use super::super::coop_native::CoopNativeRoute;

pub(super) fn request_relations(route: CoopNativeRoute, value: &Value) -> bool {
    let Some(kind) = route.request_kind() else {
        return value["kind"] == "observation";
    };
    if value["kind"].as_str() != Some(kind)
        || value["status"].is_object()
        || !value["status"].is_null()
        || !value["observation"].is_null()
        || !value["effect"].is_null()
        || !value["receipt"].is_null()
        || !value["catalog"].is_null()
    {
        return false;
    }
    match route {
        CoopNativeRoute::LegalCatalog => value["operation_id"].is_null(),
        CoopNativeRoute::LocalAction => value["action"].is_object() && value["vote"].is_null(),
        CoopNativeRoute::SharedVote => value["vote"].is_object() && value["action"].is_null(),
        CoopNativeRoute::Rejoin => value["recovery"]["kind"] == "rejoin",
        CoopNativeRoute::Recover => value["recovery"]["kind"] == "reconcile",
        CoopNativeRoute::Observation => false,
    }
}

pub(super) fn response_relations(
    route: CoopNativeRoute,
    request: Option<&Value>,
    status: u16,
    value: &Value,
) -> bool {
    let Some(request) = request else {
        return route == CoopNativeRoute::Observation
            && status == 200
            && value["operation_id"].is_null()
            && value["actor_peer"].is_null()
            && value["expected_host_generation"].is_null()
            && value["status"].is_null()
            && value["observation"].is_object()
            && value["effect"].is_null()
            && value["recovery"].is_null()
            && value["catalog"].is_null()
            && value["receipt"].is_null();
    };

    if !matches!(status, 200 | 409) {
        return false;
    }
    if status == 409 && value["status"] != "rejected" {
        return false;
    }
    if status == 200 && value["status"] == "rejected" {
        return false;
    }
    match route {
        CoopNativeRoute::LegalCatalog => {
            value["operation_id"].is_null()
                && value["actor_peer"] == request["actor_peer"]
                && value["expected_host_generation"] == request["expected_host_generation"]
                && value["observation"].is_object()
                && value["catalog"].is_object()
                && value["catalog"]["actor_peer"] == value["actor_peer"]
                && value["catalog"]["host_generation"] == value["observation"]["host_generation"]
                && value["catalog"]["host_generation"] == request["expected_host_generation"]
                && catalog_relations(&value["catalog"])
                && value["status"].is_null()
                && value["effect"].is_null()
                && value["recovery"].is_null()
                && value["receipt"].is_null()
        }
        CoopNativeRoute::LocalAction | CoopNativeRoute::SharedVote => {
            effect_response_relations(request, value)
        }
        CoopNativeRoute::Rejoin | CoopNativeRoute::Recover => {
            recovery_response_relations(route, request, value)
        }
        CoopNativeRoute::Observation => false,
    }
}

fn effect_response_relations(request: &Value, value: &Value) -> bool {
    let Some(operation_id) = request["operation_id"].as_str() else {
        return false;
    };
    let Some(observation) = value["observation"].as_object() else {
        return false;
    };
    let Some(receipt) = value["receipt"].as_object() else {
        return false;
    };
    let Some(status) = value["status"].as_str() else {
        return false;
    };
    value["operation_id"].as_str() == Some(operation_id)
        && receipt["operation_id"].as_str() == Some(operation_id)
        && receipt["status"] == value["status"]
        && receipt_observation_relations(observation, receipt)
        && match status {
            "settled" => settled_effect_relations(request, value, receipt),
            "accepted" | "unknown" => {
                receipt["before_host_generation"] == request["expected_host_generation"]
                    && receipt["before_host_generation"] == observation["host_generation"]
                    && receipt["after_host_generation"].is_null()
            }
            "rejected" => {
                receipt["before_host_generation"] == observation["host_generation"]
                    && receipt["after_host_generation"].is_null()
            }
            _ => false,
        }
}

fn settled_effect_relations(
    request: &Value,
    value: &Value,
    receipt: &serde_json::Map<String, Value>,
) -> bool {
    let expected = request["expected_host_generation"].as_u64();
    let Some(effect) = value["effect"].as_object() else {
        return false;
    };
    effect["operation_id"].as_str() == request["operation_id"].as_str()
        && effect["from_generation"].as_u64() == expected
        && effect["to_generation"] == value["observation"]["host_generation"]
        && effect["to_generation"].as_u64() > effect["from_generation"].as_u64()
        && effect["state_digest"] == value["observation"]["state_digest"]
        && effect["authority_id"] == value["observation"]["authority_id"]
        && effect["authority_epoch"] == value["observation"]["host_authority_epoch"]
        && effect["checkpoint_id"] == value["observation"]["checkpoint_id"]
        && receipt["before_host_generation"] == effect["from_generation"]
        && receipt["after_host_generation"] == effect["to_generation"]
}

fn recovery_response_relations(route: CoopNativeRoute, request: &Value, value: &Value) -> bool {
    let Some(operation_id) = request["operation_id"].as_str() else {
        return false;
    };
    let Some(recovery_kind) = value["recovery"]["kind"].as_str() else {
        return false;
    };
    if value["operation_id"].as_str() != Some(operation_id) {
        return false;
    }
    match value["status"].as_str() {
        // The schema also admits the bodyful recovery request shape under
        // `recovery_response` for the recover route. It is valid input to
        // `validate_request`, but a downstream response must carry an
        // outcome so an echoed request cannot be surfaced as success.
        None => false,
        Some("unknown") => {
            let Some(observation) = value["observation"].as_object() else {
                return false;
            };
            let Some(receipt) = value["receipt"].as_object() else {
                return false;
            };
            ((route == CoopNativeRoute::Rejoin && recovery_kind == "rejoin")
                || (route == CoopNativeRoute::Recover && recovery_kind == "reconcile"))
                && receipt["operation_id"].as_str() == Some(operation_id)
                && (receipt["status"] == "accepted" || receipt["status"] == "unknown")
                && receipt_observation_relations(observation, receipt)
                && receipt["before_host_generation"] == observation["host_generation"]
                && match receipt["status"].as_str() {
                    // A pending rejoin may be accepted without advancing the
                    // host, which the canonical producer witness represents
                    // with an explicit same-generation after fence.
                    Some("accepted") if recovery_kind == "rejoin" => {
                        receipt["before_host_generation"] == request["expected_host_generation"]
                            && receipt["after_host_generation"] == observation["host_generation"]
                    }
                    // Reconciliation and unresolved receipts cannot claim a
                    // host transition until a settled recovery response.
                    Some("accepted") | Some("unknown") => {
                        receipt["after_host_generation"].is_null()
                    }
                    _ => false,
                }
        }
        Some("settled" | "rejected") => {
            let Some(observation) = value["observation"].as_object() else {
                return false;
            };
            let Some(receipt) = value["receipt"].as_object() else {
                return false;
            };
            recovery_kind == "reconcile"
                && receipt["operation_id"].as_str() == Some(operation_id)
                && receipt["status"] == value["status"]
                && receipt_observation_relations(observation, receipt)
                && (value["status"] != "settled"
                    || receipt["after_host_generation"] == observation["host_generation"])
        }
        Some(_) => false,
    }
}

fn receipt_observation_relations(
    observation: &serde_json::Map<String, Value>,
    receipt: &serde_json::Map<String, Value>,
) -> bool {
    receipt["state_digest"] == observation["state_digest"]
        && receipt["authority_id"] == observation["authority_id"]
        && receipt["authority_epoch"] == observation["host_authority_epoch"]
        && receipt["checkpoint_id"] == observation["checkpoint_id"]
}

fn catalog_relations(catalog: &Value) -> bool {
    let Some(object) = catalog.as_object() else {
        return false;
    };
    let Some(actor_peer) = object.get("actor_peer").and_then(Value::as_str) else {
        return false;
    };
    let Some(actions) = object.get("actions").and_then(Value::as_array) else {
        return false;
    };
    let Some(votes) = object.get("votes").and_then(Value::as_array) else {
        return false;
    };
    let mut action_ids = BTreeSet::new();
    if actions.iter().any(|action| {
        action
            .get("action_id")
            .and_then(Value::as_str)
            .is_none_or(|action_id| !action_ids.insert(action_id))
    }) {
        return false;
    }
    let mut vote_ids = BTreeSet::new();
    votes.iter().all(|vote| {
        let voter_matches = vote.get("voter_peer").and_then(Value::as_str) == Some(actor_peer);
        let vote_id = vote.get("proposal_id").and_then(Value::as_str);
        let choice = vote.get("choice").and_then(Value::as_str);
        voter_matches
            && vote_id
                .zip(choice)
                .is_some_and(|(proposal_id, choice)| vote_ids.insert((proposal_id, choice)))
    })
}
