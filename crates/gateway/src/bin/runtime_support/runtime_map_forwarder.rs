// SPDX-License-Identifier: MIT

use super::runtime_map::RuntimeMapRoute;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

const SCHEMA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/runtime-map-v1/schema.json"
));
const MAP_PROTOCOL_VERSION: &str = "runtime-map-v1";
const MAP_SCHEMA_DIGEST: &str = "6340f3cbe6c1b5728144fe89fdfdf8645acf2f59a77c0e0c30ebfeafc77515d8";
const MAP_RESPONSE_KIND: &str = "snapshot_response";
const MAP_ARTIFACT: &str = "sts2-protocol/runtime-map-v1";
const MAP_SCHEMA_SOURCE: &str = "schemas/runtime-map-v1.schema.json";
const MAP_GENERATOR: &str = "hand-authored";
pub(crate) const MAX_MAP_NODES: usize = 256;
pub(crate) const MAX_MAP_EDGES: usize = 1_024;
pub(crate) const MAX_MAP_BINDINGS: usize = 256;
pub(crate) const MAX_MAP_RESPONSE_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeMapForwarder {
    max_response_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeMapForwardError {
    ResponseOversized,
    ResponseMalformed,
}

impl RuntimeMapForwarder {
    pub(crate) const fn new(max_response_bytes: usize) -> Self {
        Self { max_response_bytes }
    }

    /// Validate the complete, versioned read-only map response before it leaves
    /// the gateway. The schema is consumed from the copied protocol artifact;
    /// identity fencing and graph relations remain gateway-owned checks.
    pub(crate) fn validate_response(
        self,
        route: RuntimeMapRoute,
        headers: &BTreeMap<String, String>,
        body: &[u8],
    ) -> Result<Value, RuntimeMapForwardError> {
        if body.len() > self.max_response_bytes || body.len() > MAX_MAP_RESPONSE_BYTES {
            return Err(RuntimeMapForwardError::ResponseOversized);
        }
        let value = super::strict_json::parse(body)
            .map_err(|_| RuntimeMapForwardError::ResponseMalformed)?;
        if route != RuntimeMapRoute::Snapshot
            || !validate_schema(&value)
            || !validate_headers(&value, headers)
            || !validate_relations(&value)
        {
            return Err(RuntimeMapForwardError::ResponseMalformed);
        }
        Ok(value)
    }
}

fn validate_schema(value: &Value) -> bool {
    static VALIDATOR: OnceLock<Option<jsonschema::Validator>> = OnceLock::new();
    let validator = VALIDATOR
        .get_or_init(|| {
            let schema: Value = serde_json::from_str(SCHEMA).ok()?;
            jsonschema::validator_for(&schema).ok()
        })
        .as_ref();
    value.get("protocol_version").and_then(Value::as_str) == Some(MAP_PROTOCOL_VERSION)
        && value.get("schema_digest").and_then(Value::as_str) == Some(MAP_SCHEMA_DIGEST)
        && validator.is_some_and(|validator| validator.is_valid(value))
}

fn validate_headers(value: &Value, headers: &BTreeMap<String, String>) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    if object.get("kind").and_then(Value::as_str) != Some(MAP_RESPONSE_KIND) {
        return false;
    }
    let provenance = object.get("provenance").and_then(Value::as_object);
    if provenance.is_none_or(|provenance| {
        provenance.len() != 3
            || provenance.get("artifact").and_then(Value::as_str) != Some(MAP_ARTIFACT)
            || provenance.get("source").and_then(Value::as_str) != Some(MAP_SCHEMA_SOURCE)
            || provenance.get("generator").and_then(Value::as_str) != Some(MAP_GENERATOR)
    }) {
        return false;
    }
    for (field, header) in [
        ("correlation_id", "x-sts2-correlation-id"),
        ("instance_id", "x-sts2-instance-id"),
        ("session_id", "x-sts2-session-id"),
        ("lease_id", "x-sts2-lease-id"),
    ] {
        if object.get(field).and_then(Value::as_str) != headers.get(header).map(String::as_str) {
            return false;
        }
    }
    let epoch = headers
        .get("x-sts2-lease-epoch")
        .and_then(|value| value.parse::<u64>().ok());
    object.get("lease_epoch").and_then(Value::as_u64) == epoch
        && object.get("snapshot").is_some_and(|snapshot| {
            snapshot.get("generation").and_then(Value::as_u64)
                == object.get("generation").and_then(Value::as_u64)
        })
}

fn validate_relations(value: &Value) -> bool {
    let Some(snapshot) = value.get("snapshot").and_then(Value::as_object) else {
        return false;
    };
    let Some(nodes) = snapshot.get("nodes").and_then(Value::as_array) else {
        return false;
    };
    if nodes.len() > MAX_MAP_NODES {
        return false;
    }
    let mut node_ids = BTreeSet::new();
    let mut visited_nodes = BTreeSet::new();
    for node in nodes {
        let Some(node) = node.as_object() else {
            return false;
        };
        let Some(id) = node.get("id").and_then(Value::as_str) else {
            return false;
        };
        let (Some(_row), Some(_column)) = (
            node.get("row").and_then(Value::as_i64),
            node.get("column").and_then(Value::as_i64),
        ) else {
            return false;
        };
        if !node_ids.insert(id) {
            return false;
        }
        if node.get("visited").and_then(Value::as_bool) == Some(true) {
            visited_nodes.insert(id);
        }
    }
    let mut adjacency = vec![Vec::new(); nodes.len()];
    let index = node_ids
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, index))
        .collect::<BTreeMap<_, _>>();
    let Some(edges) = snapshot.get("edges").and_then(Value::as_array) else {
        return false;
    };
    if edges.len() > MAX_MAP_EDGES {
        return false;
    }
    let mut edge_ids = BTreeSet::new();
    for edge in edges {
        let Some(edge) = edge.as_object() else {
            return false;
        };
        let (Some(from), Some(to)) = (
            edge.get("from").and_then(Value::as_str),
            edge.get("to").and_then(Value::as_str),
        ) else {
            return false;
        };
        let (Some(&from_index), Some(&to_index)) = (index.get(from), index.get(to)) else {
            return false;
        };
        if from == to || !edge_ids.insert((from, to)) {
            return false;
        }
        adjacency[from_index].push(to_index);
    }
    if has_cycle(&adjacency) {
        return false;
    }
    let Some(position) = snapshot.get("position").and_then(Value::as_object) else {
        return false;
    };
    let current_node = position
        .get("node_id")
        .and_then(Value::as_str)
        .filter(|_| position.get("kind").and_then(Value::as_str) == Some("current"));
    let position_valid = match position.get("kind").and_then(Value::as_str) {
        Some("current") => position
            .get("node_id")
            .and_then(Value::as_str)
            .is_some_and(|id| node_ids.contains(id) && visited_nodes.contains(id)),
        Some("pre_start") => snapshot
            .get("history")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty),
        Some("unavailable") => true,
        _ => false,
    };
    position_valid
        && validate_ids(snapshot, &node_ids, &visited_nodes)
        && validate_bindings(snapshot, &node_ids, current_node)
        && validate_availability_shape(snapshot)
}

fn validate_ids(
    snapshot: &Map<String, Value>,
    node_ids: &BTreeSet<&str>,
    visited_nodes: &BTreeSet<&str>,
) -> bool {
    let Some(history) = snapshot.get("history").and_then(Value::as_array) else {
        return false;
    };
    let Some(terminal_ids) = snapshot.get("terminal_node_ids").and_then(Value::as_array) else {
        return false;
    };
    let mut history_seen = BTreeSet::new();
    if history.iter().any(|id| {
        id.as_str().is_none_or(|id| {
            !node_ids.contains(id) || !visited_nodes.contains(id) || !history_seen.insert(id)
        })
    }) {
        return false;
    }
    let mut terminal_seen = BTreeSet::new();
    terminal_ids.iter().all(|id| {
        id.as_str()
            .is_some_and(|id| node_ids.contains(id) && terminal_seen.insert(id))
    })
}

fn validate_bindings(
    snapshot: &Map<String, Value>,
    node_ids: &BTreeSet<&str>,
    current_node: Option<&str>,
) -> bool {
    let Some(bindings) = snapshot.get("bindings").and_then(Value::as_array) else {
        return false;
    };
    if bindings.len() > MAX_MAP_BINDINGS {
        return false;
    }
    let mut graph_ids = BTreeSet::new();
    let mut host_ids = BTreeSet::new();
    let mut option_ids = BTreeSet::new();
    bindings.iter().all(|binding| {
        let Some(binding) = binding.as_object() else {
            return false;
        };
        let (Some(graph_id), Some(host_id), Some(action)) = (
            binding.get("graph_node_id").and_then(Value::as_str),
            binding.get("host_action_id").and_then(Value::as_str),
            binding.get("action").and_then(Value::as_object),
        ) else {
            return false;
        };
        let action_node = action.get("node_id").and_then(Value::as_str);
        node_ids.contains(graph_id)
            && current_node != Some(graph_id)
            && graph_ids.insert(graph_id)
            && host_ids.insert(host_id)
            && option_ids.insert(action_node.unwrap_or_default())
            && host_id != graph_id
            && action.get("kind").and_then(Value::as_str) == Some("select_map_node")
            && action_node.is_some()
    })
}

fn validate_availability_shape(snapshot: &Map<String, Value>) -> bool {
    let available = snapshot.get("availability").and_then(Value::as_str) == Some("available");
    let complete = snapshot.get("completeness").and_then(Value::as_str) == Some("complete");
    let reason_present = snapshot
        .get("reason")
        .is_some_and(|reason| !reason.is_null());
    if reason_present != (!available || !complete) || (!available && complete) {
        return false;
    }
    !available
        || (snapshot
            .get("map_instance_id")
            .is_some_and(|value| !value.is_null())
            && snapshot.get("act_id").is_some_and(|value| !value.is_null())
            && snapshot
                .get("scope_id")
                .is_some_and(|value| !value.is_null()))
}

fn has_cycle(adjacency: &[Vec<usize>]) -> bool {
    fn visit(index: usize, adjacency: &[Vec<usize>], colors: &mut [u8]) -> bool {
        colors[index] = 1;
        for &next in &adjacency[index] {
            if colors[next] == 1 || (colors[next] == 0 && visit(next, adjacency, colors)) {
                return true;
            }
        }
        colors[index] = 2;
        false
    }
    let mut colors = vec![0_u8; adjacency.len()];
    (0..adjacency.len()).any(|index| colors[index] == 0 && visit(index, adjacency, &mut colors))
}

#[cfg(test)]
#[path = "runtime_map_forwarder_tests.rs"]
mod tests;
