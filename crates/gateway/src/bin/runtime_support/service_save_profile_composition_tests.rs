// SPDX-License-Identifier: MIT

use super::*;
use std::io::{Read, Write};
use super::test_support::{authenticated_request, test_service};
use sts2_gateway::{
    InMemoryLaunchProfileBindingPort, InMemorySaveProfileRecordStore, InMemoryUserDataPort,
    InMemoryUserDataRecordStore, LAUNCH_PROFILE_CONTRACT, LAUNCH_PROFILE_ID, LaunchProfileBinding,
    LaunchProfileBindingError, LaunchProfileBindingPort, SaveProfileAuthority, UserDataIdentity,
};

use super::service_save_profile::SaveProfileRuntime;
use super::service_save_profile_composition::{
    SaveProfileActiveRun, SaveProfileDependencies,
};

const DIGEST: &str =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// Launch-profile adapter that refuses every binding, standing in for an unaccepted provider.
#[derive(Default)]
struct RefusingBindingPort;

impl LaunchProfileBindingPort for RefusingBindingPort {
    fn bind_disposable(
        &mut self,
        _user_data: UserDataIdentity,
    ) -> Result<LaunchProfileBinding, LaunchProfileBindingError> {
        Err(LaunchProfileBindingError::Profile)
    }
}

fn dependencies(
    launch_profile: Option<Box<dyn LaunchProfileBindingPort + Send>>,
) -> SaveProfileDependencies {
    match launch_profile {
        Some(launch_profile) => SaveProfileDependencies {
            intents: Some(Box::new(InMemorySaveProfileRecordStore::default())),
            allocation_port: Some(Box::new(InMemoryUserDataPort::default())),
            allocation_intents: Some(Box::new(InMemoryUserDataRecordStore::default())),
            launch_profile: Some(launch_profile),
        },
        None => SaveProfileDependencies {
            intents: Some(Box::new(InMemorySaveProfileRecordStore::default())),
            allocation_port: None,
            allocation_intents: None,
            launch_profile: None,
        },
    }
}

fn inject(
    service: &mut RuntimeService,
    address: &str,
    active_run: Option<bool>,
    dependencies: SaveProfileDependencies,
) -> Result<(), String> {
    let authority = SaveProfileAuthority {
        instance_id: service.config.instance_id.clone(),
        caller_id: service.config.caller_id.clone(),
        session_id: service.config.session_id.clone(),
        lease_id: service.config.lease_id.clone(),
        lease_epoch: service.config.lease_epoch,
        expires_at_millis: None,
    };
    service.save_profile = SaveProfileRuntime::with_dependencies(
        true,
        address,
        &service.config.mod_token,
        authority,
        service.config.operation_capacity,
        dependencies,
    )?;
    service.save_profile_active_run = SaveProfileActiveRun {
        governed: active_run,
    };
    Ok(())
}

fn closed_listener() -> Result<(TcpListener, String), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    Ok((listener, address.to_string()))
}

fn create_response(operation_id: &str, identity: u64) -> Vec<u8> {
    format!(
        r#"{{"status":"settled","instance_id":"instance-1","caller_id":"harness","session_id":"session-1","lease_id":"lease-1","lease_epoch":1,"correlation_id":"corr-create","operation_id":"{operation_id}","baseline":{{"identity":"baseline-1","digest":"{DIGEST}"}},"user_data":{{"identity":{identity},"provenance":{{"owner":"gateway","instance_id":"instance-1","operation_id":"{operation_id}","contract":"{LAUNCH_PROFILE_CONTRACT}"}},"baseline":null}}}}"#
    )
    .into_bytes()
}

fn serve(listener: TcpListener, response: Vec<u8>) -> thread::JoinHandle<Result<String, String>> {
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let read = stream.read(&mut buffer).map_err(|error| error.to_string())?;
            if read == 0 {
                return Err(String::from("downstream closed before request"));
            }
            request.extend_from_slice(&buffer[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                let content_length = content_length(&request)?;
                let head = request
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .map(|position| position + 4)
                    .unwrap_or(request.len());
                if request.len() >= head + content_length {
                    break;
                }
            }
        }
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.len()
        );
        stream
            .write_all(header.as_bytes())
            .and_then(|_| stream.write_all(&response))
            .map_err(|error| error.to_string())?;
        String::from_utf8(request).map_err(|error| error.to_string())
    })
}

fn content_length(request: &[u8]) -> Result<usize, String> {
    let text = String::from_utf8_lossy(request);
    for line in text.lines() {
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            return value
                .trim()
                .parse::<usize>()
                .map_err(|error| error.to_string());
        }
    }
    Ok(0)
}

fn mutation_request(path: &str, operation_id: &str, body: Vec<u8>) -> HttpRequest {
    let mut request = authenticated_request(path);
    request.method = String::from("POST");
    request
        .headers
        .insert(String::from("content-type"), String::from("application/json"));
    request
        .headers
        .insert(String::from("x-mcp-request-id"), operation_id.to_owned());
    request
        .headers
        .insert(String::from("x-sts2-correlation-id"), String::from("corr-create"));
    request.body = body;
    request
}

fn select_body() -> Vec<u8> {
    format!(
        r#"{{"profile_id":"slot-1","baseline":{{"identity":"baseline-1","digest":"{DIGEST}"}}}}"#
    )
    .into_bytes()
}

fn error_code(body: &[u8]) -> Result<String, String> {
    let value: Value = serde_json::from_slice(body).map_err(|error| error.to_string())?;
    value
        .get("error_code")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| String::from("error code missing"))
}

#[test]
fn unprovisioned_composition_refuses_mutations_before_any_forwarding() -> Result<(), String> {
    let mut service = test_service()?;
    let (listener, address) = closed_listener()?;
    inject(
        &mut service,
        &address,
        None,
        SaveProfileDependencies::unavailable(),
    )?;
    let select = mutation_request(
        "/v1/instances/instance-1/save-profile/select",
        "op-select",
        select_body(),
    );
    let (status, body) = service.handle_request(&select);
    assert_eq!(status, 503);
    assert_eq!(error_code(&body)?, "save_profile_active_run_unavailable");

    let create = mutation_request(
        "/v1/instances/instance-1/save-profile/create-disposable",
        "op-create",
        b"{}".to_vec(),
    );
    let (status, body) = service.handle_request(&create);
    assert_eq!(status, 503);
    assert_eq!(error_code(&body)?, "save_profile_active_run_unavailable");
    assert!(matches!(
        listener.accept(),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
    ));
    Ok(())
}

#[test]
fn missing_durable_intents_refuses_every_mutation_before_dispatch() -> Result<(), String> {
    let mut service = test_service()?;
    inject(
        &mut service,
        "127.0.0.1:1",
        Some(false),
        SaveProfileDependencies {
            intents: None,
            allocation_port: None,
            allocation_intents: None,
            launch_profile: None,
        },
    )?;
    let select = mutation_request(
        "/v1/instances/instance-1/save-profile/select",
        "op-select",
        select_body(),
    );
    let (status, body) = service.handle_request(&select);
    assert_eq!(status, 503);
    assert_eq!(error_code(&body)?, "save_profile_persistence_unavailable");

    let create = mutation_request(
        "/v1/instances/instance-1/save-profile/create-disposable",
        "op-create",
        b"{}".to_vec(),
    );
    let (status, body) = service.handle_request(&create);
    assert_eq!(status, 503);
    assert_eq!(error_code(&body)?, "save_profile_persistence_unavailable");
    Ok(())
}

#[test]
fn durable_intents_without_an_allocation_adapter_report_provisioning_unavailable()
-> Result<(), String> {
    let mut service = test_service()?;
    let (listener, address) = closed_listener()?;
    inject(&mut service, &address, Some(false), dependencies(None))?;
    let create = mutation_request(
        "/v1/instances/instance-1/save-profile/create-disposable",
        "op-create",
        b"{}".to_vec(),
    );
    let (status, body) = service.handle_request(&create);
    assert_eq!(status, 503);
    assert_eq!(error_code(&body)?, "save_profile_provisioning_unavailable");
    assert!(matches!(
        listener.accept(),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
    ));
    Ok(())
}

#[test]
fn injected_allocation_forwards_the_approved_launch_binding() -> Result<(), String> {
    let (listener, address) = closed_listener()?;
    listener
        .set_nonblocking(false)
        .map_err(|error| error.to_string())?;
    let server = serve(listener, create_response("op-create", 1));
    let mut service = test_service()?;
    inject(
        &mut service,
        &address,
        Some(false),
        dependencies(Some(Box::new(
            InMemoryLaunchProfileBindingPort::default(),
        ))),
    )?;
    let create = mutation_request(
        "/v1/instances/instance-1/save-profile/create-disposable",
        "op-create",
        b"{}".to_vec(),
    );
    let (status, body) = service.handle_request(&create);
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let value: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    assert_eq!(value["status"], "settled");
    assert_eq!(value["user_data"]["identity"], 1);

    let wire = server
        .join()
        .map_err(|_| String::from("synthetic downstream panicked"))??;
    assert!(wire.starts_with("POST /api/v1/save-profile/create-disposable HTTP/1.1\r\n"));
    let body = wire
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .ok_or_else(|| String::from("downstream body missing"))?;
    let forwarded: Value = serde_json::from_str(body).map_err(|error| error.to_string())?;
    assert_eq!(forwarded["user_data"]["identity"], 1);
    assert_eq!(forwarded["launch_profile"]["contract"], LAUNCH_PROFILE_CONTRACT);
    assert_eq!(forwarded["launch_profile"]["profile_id"], LAUNCH_PROFILE_ID);
    assert_eq!(forwarded["launch_profile"]["user_data"], 1);
    Ok(())
}

#[test]
fn refusing_launch_profile_port_fails_closed_without_forwarding() -> Result<(), String> {
    let mut service = test_service()?;
    let (listener, address) = closed_listener()?;
    inject(
        &mut service,
        &address,
        Some(false),
        dependencies(Some(Box::new(RefusingBindingPort))),
    )?;
    let create = mutation_request(
        "/v1/instances/instance-1/save-profile/create-disposable",
        "op-create",
        b"{}".to_vec(),
    );
    let (status, body) = service.handle_request(&create);
    assert_eq!(status, 503);
    assert_eq!(error_code(&body)?, "save_profile_provisioning_failed");
    assert!(matches!(
        listener.accept(),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
    ));
    Ok(())
}
