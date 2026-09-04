// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::super::http::HttpRequest;
use super::{RuntimeV3GameplayProxy, validation, wire};

fn request(correlation: &str) -> Result<HttpRequest, String> {
    let body = json!({
        "protocol_version": "runtime-v3-gameplay",
        "schema_digest": super::RUNTIME_V3_GAMEPLAY_SCHEMA_DIGEST,
        "provenance": {"artifact": super::RUNTIME_V3_GAMEPLAY_ARTIFACT,
            "source": super::RUNTIME_V3_GAMEPLAY_SCHEMA_SOURCE,
            "generator": "hand-authored"},
        "correlation_id": correlation, "instance_id": "instance-1", "session_id": "session-1",
        "lease_id": "lease-1", "lease_epoch": 1, "generation": 4,
        "kind": "action_request", "operation_id": "op-1", "observation": null,
        "action": {"action_id": "play_card", "card_index": 0, "target_id": null},
        "status": null, "error_code": null, "effect_witness": null
    });
    Ok(HttpRequest {
        method: "POST".into(),
        path: "/unused".into(),
        headers: BTreeMap::from([("x-sts2-correlation-id".into(), correlation.into())]),
        body: serde_json::to_vec(&body).map_err(|error| error.to_string())?,
    })
}

fn response(status: &str, correlation: &str, generation: u64) -> Result<Vec<u8>, String> {
    let mut value: Value =
        serde_json::from_slice(&request(correlation)?.body).map_err(|error| error.to_string())?;
    value["kind"] = json!("action_response");
    value["status"] = json!(status);
    value["generation"] = json!(generation);
    if status == "unknown" {
        value["error_code"] = json!("sts2.runtime/operation_in_progress");
    } else {
        value["observation"] = json!({"combat_phase": "combat/player_turn", "turn_index": 2,
            "host_ready": true, "generation": generation, "hand_count": 2, "energy": 2,
            "draw_pile_count": 2, "discard_pile_count": 2, "exhaust_pile_count": 0, "enemies": []});
    }
    if status == "settled" {
        value["effect_witness"] = json!({"kind": "play_card_settled", "generation": generation,
            "card_index": 0, "target_id": null});
    }
    serde_json::to_vec(&value).map_err(|error| error.to_string())
}

fn proxy(address: &str) -> RuntimeV3GameplayProxy {
    RuntimeV3GameplayProxy::new(
        address,
        "synthetic-token",
        "instance-1",
        "caller-1",
        "session-1",
        "lease-1",
        1,
        8,
    )
}

type ServerHandle = thread::JoinHandle<Result<Vec<String>, String>>;

fn server(responses: Vec<Vec<u8>>) -> Result<(String, ServerHandle), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let handle = thread::spawn(move || {
        let mut paths = Vec::new();
        for response in responses {
            let start = Instant::now();
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && start.elapsed() < Duration::from_secs(3) =>
                    {
                        thread::sleep(Duration::from_millis(1))
                    }
                    Err(error) => return Err(error.to_string()),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .map_err(|error| error.to_string())?;
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .map_err(|error| error.to_string())?;
            let mut bytes = Vec::new();
            let mut buffer = [0_u8; 2048];
            loop {
                let count = stream
                    .read(&mut buffer)
                    .map_err(|error| error.to_string())?;
                if count == 0 || bytes.len() > 32768 {
                    return Err("incomplete request".into());
                }
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("Content-Length: "))
                        .and_then(|value| value.parse::<usize>().ok())
                        .ok_or("missing length")?;
                    if bytes.len() >= end + 4 + length {
                        paths.push(
                            headers
                                .lines()
                                .next()
                                .ok_or("missing request line")?
                                .to_owned(),
                        );
                        break;
                    }
                }
            }
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                response.len()
            );
            stream
                .write_all(header.as_bytes())
                .and_then(|()| stream.write_all(&response))
                .map_err(|error| error.to_string())?;
        }
        Ok(paths)
    });
    Ok((address, handle))
}

#[test]
fn accepted_and_unknown_receipts_remain_reconcilable_and_replay_rebinds() -> Result<(), String> {
    let (address, handle) = server(vec![
        response("accepted", "action", 4)?,
        response("unknown", "poll-1", 4)?,
        response("settled", "poll-2", 5)?,
    ])?;
    let mut proxy = proxy(&address);
    let action = request("action")?;
    let (status, _) = proxy.action(&action, "instance-1", "session-1", "lease-1", 1);
    assert_eq!(status, 200);
    for (correlation, expected) in [
        ("poll-1", "unknown"),
        ("poll-2", "settled"),
        ("poll-3", "settled"),
    ] {
        let (status, body) = proxy.operation(
            &request(correlation)?,
            "op-1",
            "instance-1",
            "session-1",
            "lease-1",
            1,
        );
        let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
        assert_eq!(status, 200);
        assert_eq!(value["status"], expected);
        assert_eq!(value["correlation_id"], correlation);
    }
    let mut retry = request("retry")?;
    let value: Value = serde_json::from_slice(&retry.body).map_err(|error| error.to_string())?;
    retry.body = serde_json::to_vec_pretty(&value).map_err(|error| error.to_string())?;
    let (status, body) = proxy.action(&retry, "instance-1", "session-1", "lease-1", 1);
    assert_eq!(status, 200);
    let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["correlation_id"], "retry");
    assert_eq!(value["status"], "settled");
    let mut changed: Value =
        serde_json::from_slice(&retry.body).map_err(|error| error.to_string())?;
    changed["action"]["card_index"] = json!(1);
    retry.body = serde_json::to_vec(&changed).map_err(|error| error.to_string())?;
    assert_eq!(
        proxy
            .action(&retry, "instance-1", "session-1", "lease-1", 1)
            .0,
        409
    );
    let paths = handle.join().map_err(|_| "synthetic server panicked")??;
    assert_eq!(
        paths,
        [
            "POST /api/v3/runtime/action HTTP/1.1",
            "GET /api/v3/runtime/operations/op-1 HTTP/1.1",
            "GET /api/v3/runtime/operations/op-1 HTTP/1.1"
        ]
    );
    Ok(())
}

#[test]
fn prewrite_unavailable_does_not_poison_operation_capacity() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    drop(listener);
    let mut proxy = proxy(&address.to_string());
    assert_eq!(
        proxy
            .action(&request("action")?, "instance-1", "session-1", "lease-1", 1)
            .0,
        503
    );
    assert!(proxy.operations.is_empty());
    Ok(())
}

#[test]
fn settled_response_requires_a_successor_and_exact_action() -> Result<(), String> {
    let request = request("action")?;
    let action = validation::parse_action_request(
        &request.body,
        "instance-1",
        "session-1",
        "lease-1",
        1,
        "action",
    )
    .map_err(str::to_owned)?;
    for (generation, valid) in [(3, false), (4, false), (5, true)] {
        assert_eq!(
            validation::validate_result_response(
                &response("settled", "action", generation)?,
                "instance-1",
                "session-1",
                "lease-1",
                1,
                "action",
                "op-1",
                &action
            )
            .is_ok(),
            valid
        );
    }
    Ok(())
}

#[test]
fn duplicate_keys_are_rejected_at_every_depth() -> Result<(), String> {
    assert!(wire::decode(br#"{"a":1,"a":2}"#).is_err());
    assert!(wire::decode(br#"{"a":[{"x":1,"x":2}]}"#).is_err());
    assert!(wire::decode(br#"{"a":{"x":1,"\u0078":2}}"#).is_err());
    assert_eq!(
        wire::decode(br#"[null,true,-1,2,1.5,"x"]"#).map_err(|error| error.to_string())?,
        json!([null, true, -1, 2, 1.5, "x"])
    );
    let original = String::from_utf8(request("action")?.body).map_err(|error| error.to_string())?;
    let duplicate = original.replacen("{", "{\"generation\":9,", 1);
    assert!(
        validation::parse_action_request(
            duplicate.as_bytes(),
            "instance-1",
            "session-1",
            "lease-1",
            1,
            "action"
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn malformed_action_reply_is_retained_for_read_only_reconciliation() -> Result<(), String> {
    let (address, handle) = server(vec![b"{}".to_vec(), response("settled", "poll", 5)?])?;
    let mut proxy = proxy(&address);
    assert_eq!(
        proxy
            .action(&request("action")?, "instance-1", "session-1", "lease-1", 1)
            .0,
        502
    );
    assert_eq!(proxy.operations.len(), 1);
    assert_eq!(
        proxy
            .action(&request("retry")?, "instance-1", "session-1", "lease-1", 1)
            .0,
        503
    );
    let (status, body) = proxy.operation(
        &request("poll")?,
        "op-1",
        "instance-1",
        "session-1",
        "lease-1",
        1,
    );
    assert_eq!(status, 200);
    assert_eq!(
        wire::decode(&body).map_err(|error| error.to_string())?["status"],
        "settled"
    );
    let paths = handle.join().map_err(|_| "synthetic server panicked")??;
    assert_eq!(paths.len(), 2);
    assert!(paths[1].starts_with("GET "));
    Ok(())
}

#[test]
fn regressed_state_is_rejected_without_rewinding_admission() -> Result<(), String> {
    let mut responses = Vec::new();
    for generation in [6, 5] {
        let mut value = wire::decode(&response("accepted", "state", generation)?)
            .map_err(|error| error.to_string())?;
        value["kind"] = json!("state_response");
        value["operation_id"] = Value::Null;
        value["action"] = Value::Null;
        value["status"] = Value::Null;
        responses.push(serde_json::to_vec(&value).map_err(|error| error.to_string())?);
    }
    let (address, handle) = server(responses)?;
    let mut proxy = proxy(&address);
    assert_eq!(
        proxy
            .state(&request("state")?, "instance-1", "session-1", "lease-1", 1)
            .0,
        200
    );
    assert_eq!(
        proxy
            .state(&request("state")?, "instance-1", "session-1", "lease-1", 1)
            .0,
        502
    );
    assert_eq!(proxy.generation, 6);
    assert_eq!(
        proxy
            .action(&request("action")?, "instance-1", "session-1", "lease-1", 1)
            .0,
        409
    );
    assert_eq!(
        handle
            .join()
            .map_err(|_| "synthetic server panicked")??
            .len(),
        2
    );
    Ok(())
}

#[test]
fn forwarding_rejects_dns_and_nonloopback_without_writing() {
    for address in ["localhost:1234", "192.0.2.1:1234"] {
        assert!(matches!(
            proxy(address).forwarder.forward_state("state"),
            Err(super::RuntimeV3GameplayTransportError::Unavailable)
        ));
    }
}
