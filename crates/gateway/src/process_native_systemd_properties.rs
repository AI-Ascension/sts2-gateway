// SPDX-License-Identifier: MIT

use super::{GenerationRecord, ProcessFault};
use std::collections::BTreeMap;

pub(in crate::process_native) fn verify(
    record: &GenerationRecord,
    properties: &mut BTreeMap<String, String>,
    empty_group: impl FnOnce(&str) -> Result<bool, ProcessFault>,
) -> Result<(), ProcessFault> {
    for (name, value) in [
        ("LoadState", "loaded"),
        ("Transient", "yes"),
        ("KillMode", "control-group"),
        ("SendSIGKILL", "yes"),
        ("Restart", "no"),
        ("Delegate", "no"),
        ("ProtectControlGroups", "yes"),
        ("RemainAfterExit", "yes"),
        ("ExitType", "cgroup"),
    ] {
        if properties.get(name).map(String::as_str) != Some(value) {
            return Err(ProcessFault::IdentityMismatch);
        }
    }
    if properties.get("Description") != Some(&description(record)) {
        return Err(ProcessFault::IdentityMismatch);
    }
    // systemd can release a terminal unit's cgroup and clear its invocation fields.
    // Only a previously bound, now empty cgroup supplies that missing evidence.
    if properties.get("ControlGroup").is_some_and(String::is_empty)
        && terminal(properties)
        && number(properties, "MainPID")? == 0
        && let (Some(group), Some(invocation)) = (&record.control_group, &record.invocation)
        && empty_group(group)?
    {
        properties.insert("ControlGroup".to_owned(), group.clone());
        if properties
            .get("InvocationID")
            .is_some_and(|value| value.is_empty() || value == "00000000000000000000000000000000")
        {
            properties.insert("InvocationID".to_owned(), invocation.clone());
        }
    }
    let invocation = properties
        .get("InvocationID")
        .ok_or(ProcessFault::InspectionFailed)?;
    if invocation.len() != 32
        || uuid::Uuid::parse_str(invocation).is_err()
        || invocation == "00000000000000000000000000000000"
        || record
            .invocation
            .as_ref()
            .is_some_and(|value| value != invocation)
    {
        return Err(ProcessFault::IdentityMismatch);
    }
    let group = properties
        .get("ControlGroup")
        .ok_or(ProcessFault::InspectionFailed)?;
    if !super::super::identity::valid_group(group, &record.unit)
        || record
            .control_group
            .as_ref()
            .is_some_and(|value| value != group)
    {
        return Err(ProcessFault::IdentityMismatch);
    }
    Ok(())
}

pub(super) fn terminal(properties: &BTreeMap<String, String>) -> bool {
    let active = properties.get("ActiveState").map(String::as_str);
    properties.get("Job").map(String::as_str) == Some("0")
        && (matches!(active, Some("inactive" | "failed"))
            || (active == Some("active")
                && properties.get("SubState").map(String::as_str) == Some("exited")))
}

pub(super) fn description(record: &GenerationRecord) -> String {
    format!("sts2-generation:{}:{}", record.key, record.binding_digest)
}

pub(super) fn parse(bytes: &[u8]) -> Result<BTreeMap<String, String>, ProcessFault> {
    let text = std::str::from_utf8(bytes).map_err(|_| ProcessFault::InspectionFailed)?;
    let mut properties = BTreeMap::new();
    for line in text.lines() {
        let (name, value) = line.split_once('=').ok_or(ProcessFault::InspectionFailed)?;
        if properties
            .insert(name.to_owned(), value.to_owned())
            .is_some()
        {
            return Err(ProcessFault::InspectionFailed);
        }
    }
    Ok(properties)
}

pub(super) fn number(
    properties: &BTreeMap<String, String>,
    name: &str,
) -> Result<u64, ProcessFault> {
    properties
        .get(name)
        .ok_or(ProcessFault::InspectionFailed)?
        .parse()
        .map_err(|_| ProcessFault::InspectionFailed)
}
