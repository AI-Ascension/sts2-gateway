// SPDX-License-Identifier: MIT

use super::test_support::{authenticated_request, test_service};
use super::{read_request, write_response};
use serde_json::Value;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

fn golden(name: &str) -> &'static [u8] {
    match name {
        "request" => include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../protocol-artifact/runtime-v4-expert-rest-action/golden/action-request.json"
        )),
        "accepted" => include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../protocol-artifact/runtime-v4-expert-rest-action/golden/action-accepted.json"
        )),
        "unknown" => include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../protocol-artifact/runtime-v4-expert-rest-action/golden/action-unknown.json"
        )),
        _ => &[],
    }
}

fn service_identity(body: &[u8]) -> Result<Vec<u8>, String> {
    let text = String::from_utf8(body.to_vec()).map_err(|error| error.to_string())?;
    let replacements = [
        ("instance:1", "instance-1"),
        ("session:1", "session-1"),
        ("lease:1", "lease-1"),
        ("corr:rest:1", "corr-state"),
        ("\"lease_epoch\": 4", "\"lease_epoch\": 1"),
    ];
    let mut output = text;
    for (from, to) in replacements {
        output = output.replace(from, to);
    }
    Ok(output.into_bytes())
}

#[test]
fn dispatch_and_reconcile_forward_their_fixed_paths() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let request_body = service_identity(golden("request"))?;
    let response_body = service_identity(golden("accepted"))?;
    let worker_request_body = request_body.clone();
    let worker_response_body = response_body.clone();
    let worker = thread::spawn(move || -> Result<Vec<String>, String> {
        let mut paths = Vec::new();
        for _ in 0..2 {
            let expires = Instant::now() + Duration::from_secs(2);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error)
                        if error.kind() == ErrorKind::WouldBlock && Instant::now() < expires =>
                    {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => return Err(error.to_string()),
                }
            };
            let forwarded = read_request(&mut stream).map_err(|error| error.to_string())?;
            paths.push(forwarded.path.clone());
            if paths.len() == 1 {
                if forwarded.method != "POST" || forwarded.body != worker_request_body {
                    return Err(String::from("dispatch request changed at gateway"));
                }
            } else if forwarded.method != "GET" || !forwarded.body.is_empty() {
                return Err(String::from("reconcile request carried a body"));
            }
            // The native callback currently emits HTTP 200 for every typed
            // outcome; the gateway maps the envelope status to its candidate
            // HTTP assignment before returning it.
            write_response(&mut stream, 200, &worker_response_body)
                .map_err(|error| error.to_string())?;
        }
        Ok(paths)
    });

    let mut service = test_service()?;
    service.config.mod_address = address;
    let mut dispatch = authenticated_request("/v4/instances/instance-1/expert-rest-action");
    dispatch.method = String::from("POST");
    dispatch.headers.insert(
        String::from("content-type"),
        String::from("application/json"),
    );
    dispatch.body = request_body.clone();
    let (status, body) = service.handle_request(&dispatch);
    if status != 202 || body != response_body {
        return Err(format!("dispatch returned {status} with unexpected body"));
    }

    let reconcile_path = "/v4/instances/instance-1/expert-rest-actions/rest-op:7:heal";
    let reconcile = authenticated_request(reconcile_path);
    let (status, body) = service.handle_request(&reconcile);
    if status != 202 || body != response_body {
        return Err(format!("reconcile returned {status} with unexpected body"));
    }
    let paths = worker
        .join()
        .map_err(|_| String::from("downstream worker panicked"))??;
    assert_eq!(
        paths,
        vec![
            String::from("/api/v4/runtime/expert-rest-action"),
            String::from("/api/v4/runtime/expert-rest-actions/rest-op:7:heal"),
        ]
    );
    Ok(())
}

#[test]
fn malformed_profile_is_rejected_before_downstream() -> Result<(), String> {
    let mut service = test_service()?;
    let mut request = authenticated_request("/v4/instances/instance-1/expert-rest-action");
    request.method = String::from("POST");
    request.headers.insert(
        String::from("content-type"),
        String::from("application/json"),
    );
    let mut value: Value =
        serde_json::from_slice(golden("request")).map_err(|error| error.to_string())?;
    value["profile"] = Value::String(String::from("expert-action"));
    request.body =
        service_identity(&serde_json::to_vec(&value).map_err(|error| error.to_string())?)?;
    let (status, body) = service.handle_request(&request);
    assert_eq!(status, 400);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?["error_code"],
        "runtime_v4_expert_rest_action_request_invalid"
    );
    Ok(())
}

#[test]
fn native_unknown_503_maps_to_canonical_bad_gateway() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    let request_body = service_identity(golden("request"))?;
    let response_body = service_identity(golden("unknown"))?;
    let worker_request_body = request_body.clone();
    let worker_response_body = response_body.clone();
    let worker = thread::spawn(move || -> Result<(), String> {
        let expires = Instant::now() + Duration::from_secs(2);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == ErrorKind::WouldBlock && Instant::now() < expires => {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) => return Err(error.to_string()),
            }
        };
        let forwarded = read_request(&mut stream).map_err(|error| error.to_string())?;
        if forwarded.method != "POST"
            || forwarded.path != "/api/v4/runtime/expert-rest-action"
            || forwarded.body != worker_request_body
        {
            return Err(String::from("forwarded unknown request changed at gateway"));
        }
        write_response(&mut stream, 503, &worker_response_body)
            .map_err(|error| error.to_string())?;
        Ok(())
    });

    let mut service = test_service()?;
    service.config.mod_address = address;
    let mut request = authenticated_request("/v4/instances/instance-1/expert-rest-action");
    request.method = String::from("POST");
    request.headers.insert(
        String::from("content-type"),
        String::from("application/json"),
    );
    request.body = request_body;
    let (status, body) = service.handle_request(&request);
    if status != 502 || body != response_body {
        return Err(format!(
            "unknown 503 returned {status} with unexpected body"
        ));
    }
    worker
        .join()
        .map_err(|_| String::from("synthetic downstream panicked"))??;
    Ok(())
}
