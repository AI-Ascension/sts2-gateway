// SPDX-License-Identifier: MIT

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use serde_json::to_vec;
use sts2_gateway::{
    RuntimeV2Action, RuntimeV2CombatPhase, RuntimeV2ForwardingPort, RuntimeV2Message,
    RuntimeV2MessageKind, RuntimeV2Metadata, RuntimeV2Observation, RuntimeV2Status,
};

use super::{HttpRuntimeV2Forwarder, RuntimeV2TransportFault};

fn state_request() -> RuntimeV2Message {
    RuntimeV2Message::state_request(
        RuntimeV2Metadata::new(),
        "corr-state",
        "instance-1",
        "session-1",
        "lease-1",
        1,
        4,
    )
}

fn state_response() -> RuntimeV2Message {
    RuntimeV2Message::state_response(
        RuntimeV2Metadata::new(),
        "corr-state",
        "instance-1",
        "session-1",
        "lease-1",
        1,
        RuntimeV2Observation::new(RuntimeV2CombatPhase::PlayerTurn, 2, true, 4),
    )
}

fn http_response(message: &RuntimeV2Message) -> Result<Vec<u8>, String> {
    let body = to_vec(message).map_err(|error| error.to_string())?;
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut response = header.into_bytes();
    response.extend_from_slice(&body);
    Ok(response)
}

fn serve_once(
    listener: TcpListener,
    response: Vec<u8>,
) -> thread::JoinHandle<Result<String, String>> {
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let read = stream
                .read(&mut buffer)
                .map_err(|error| error.to_string())?;
            if read == 0 {
                return Err(String::from("client closed before request headers"));
            }
            bytes.extend_from_slice(&buffer[..read]);
            if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
            if bytes.len() > 8 * 1024 {
                return Err(String::from("request headers exceeded test bound"));
            }
        }
        stream
            .write_all(&response)
            .map_err(|error| error.to_string())?;
        String::from_utf8(bytes).map_err(|error| error.to_string())
    })
}

#[test]
fn state_forwarding_uses_the_v2_route_and_fenced_headers() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let response = http_response(&state_response())?;
    let server = serve_once(listener, response);
    let mut forwarder = HttpRuntimeV2Forwarder::new(
        &address.to_string(),
        "mod-token",
        "instance-1",
        "caller-1",
        "session-1",
        "lease-1",
        1,
    );

    let result = forwarder
        .forward_state(state_request())
        .map_err(|error| format!("state forwarding failed: {error:?}"))?;
    assert_eq!(result.kind, RuntimeV2MessageKind::StateResponse);
    assert_eq!(result.observation.map(|value| value.generation), Some(4));
    let request = server
        .join()
        .map_err(|_| String::from("forwarder test server panicked"))??;
    if !request.starts_with("GET /api/v2/runtime/state HTTP/1.1\r\n")
        || !request.contains("Authorization: Bearer mod-token\r\n")
        || !request.contains("x-sts2-instance-id: instance-1\r\n")
        || !request.contains("x-sts2-correlation-id: corr-state\r\n")
    {
        return Err(String::from(
            "state request omitted its fixed v2 route or fence",
        ));
    }
    Ok(())
}

#[test]
fn action_forwarding_is_single_request_and_decodes_a_settled_result() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let request = RuntimeV2Message::action_request(
        RuntimeV2Metadata::new(),
        "corr-action",
        "instance-1",
        "session-1",
        "lease-1",
        1,
        4,
        "op-1",
        RuntimeV2Action::end_turn(),
    );
    let response = RuntimeV2Message::result(
        RuntimeV2Metadata::new(),
        "corr-action",
        "instance-1",
        "session-1",
        "lease-1",
        1,
        5,
        "op-1",
        RuntimeV2Action::end_turn(),
        RuntimeV2Status::Settled,
        Some(RuntimeV2Observation::new(
            RuntimeV2CombatPhase::PlayerTurn,
            3,
            true,
            5,
        )),
        None,
        Some(sts2_gateway::RuntimeV2EffectWitness::turn_end_settled(5)),
        RuntimeV2MessageKind::ActionResponse,
    );
    let server = serve_once(listener, http_response(&response)?);
    let mut forwarder = HttpRuntimeV2Forwarder::new(
        &address.to_string(),
        "mod-token",
        "instance-1",
        "caller-1",
        "session-1",
        "lease-1",
        1,
    );
    let decoded = forwarder
        .forward_runtime_v2(sts2_gateway::RuntimeV2ForwardRequest::new(request))
        .map_err(|error| format!("action forwarding failed: {error:?}"))?;
    assert_eq!(decoded.status, Some(RuntimeV2Status::Settled));
    let wire_request = server
        .join()
        .map_err(|_| String::from("forwarder test server panicked"))??;
    if !wire_request.starts_with("POST /api/v2/runtime/action HTTP/1.1\r\n") {
        return Err(String::from("action request used an unexpected route"));
    }
    Ok(())
}

#[test]
fn unreachable_mod_is_a_pre_write_unavailable_fault() -> Result<(), String> {
    let mut forwarder = HttpRuntimeV2Forwarder::new(
        "127.0.0.1:1",
        "mod-token",
        "instance-1",
        "caller-1",
        "session-1",
        "lease-1",
        1,
    );
    assert_eq!(
        forwarder.forward_state(state_request()),
        Err(RuntimeV2TransportFault::UnavailableBeforeWrite)
    );
    Ok(())
}
