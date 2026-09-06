// SPDX-License-Identifier: MIT

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use super::{HOST_FENCE_PATH, HttpRecoveryControlForwarder, RecoveryControlTransportFault};
use sts2_gateway::{MAX_RECOVERY_FRAME_BYTES, RECOVERY_SCHEMA_DIGEST};

fn frame() -> Vec<u8> {
    format!(
        r#"{{"contract":"watchdog-recovery-v1","schema_digest":"{RECOVERY_SCHEMA_DIGEST}","message_id":"message-1","correlation_id":"correlation-1","sent_at":"2026-09-06T23:00:00Z","actor":{{"principal_id":"principal-1","role":"gateway"}},"auth":{{"principal_id":"principal-1","capability":"host_fence","proof":"proof-1"}},"kind":"host_fence_request","payload":{{"boot":{{"deployment_id":"deployment-1"}}}}}}"#
    )
    .into_bytes()
}

fn response() -> Vec<u8> {
    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}"
        .to_vec()
}

#[test]
fn host_fence_uses_fixed_authenticated_route_without_old_lease_headers() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 1024];
        loop {
            let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
            if read == 0 {
                return Err(String::from("client closed before request"));
            }
            bytes.extend_from_slice(&chunk[..read]);
            if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                let header_end = bytes
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .ok_or_else(|| String::from("header terminator disappeared"))?;
                let header = String::from_utf8_lossy(&bytes[..header_end]);
                let length = header
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .ok_or_else(|| String::from("content length missing"))?
                    .parse::<usize>()
                    .map_err(|error| error.to_string())?;
                if bytes.len() >= header_end + 4 + length {
                    break;
                }
            }
        }
        stream
            .write_all(&response())
            .map_err(|error| error.to_string())?;
        String::from_utf8(bytes).map_err(|error| error.to_string())
    });
    let forwarder = HttpRecoveryControlForwarder::new(&address.to_string(), "mod-token");
    let result = forwarder
        .forward_host_fence(&frame())
        .map_err(|error| format!("host-fence forwarding failed: {error:?}"))?;
    assert_eq!(result.status, 200);
    let request = server
        .join()
        .map_err(|_| String::from("recovery server panicked"))??;
    assert!(request.starts_with(&format!("POST {HOST_FENCE_PATH} HTTP/1.1\r\n")));
    assert!(request.contains("Authorization: Bearer mod-token\r\n"));
    assert!(request.contains("Content-Type: application/json\r\n"));
    assert!(!request.contains("x-sts2-lease-id"));
    assert!(!request.contains("x-sts2-lease-epoch"));
    Ok(())
}

#[test]
fn host_fence_rejects_mixed_or_oversized_frames_before_connecting() -> Result<(), String> {
    let mut invalid = frame();
    let invalid_text = String::from_utf8(invalid)
        .map_err(|error| error.to_string())?
        .replace("host_fence_request", "bootstrap_request");
    invalid = invalid_text.into_bytes();
    assert!(matches!(
        HttpRecoveryControlForwarder::new("127.0.0.1:1", "token").forward_host_fence(&invalid),
        Err(RecoveryControlTransportFault::InvalidFrame)
    ));
    let oversized = vec![b'x'; MAX_RECOVERY_FRAME_BYTES + 1];
    assert!(matches!(
        HttpRecoveryControlForwarder::new("127.0.0.1:1", "token").forward_host_fence(&oversized),
        Err(RecoveryControlTransportFault::RequestOversized)
    ));
    Ok(())
}

#[test]
fn host_fence_rejects_invalid_mod_configuration_before_connecting() {
    assert!(matches!(
        HttpRecoveryControlForwarder::new("127.0.0.1:1", "bad token").forward_host_fence(&frame()),
        Err(RecoveryControlTransportFault::InvalidConfiguration)
    ));
}
