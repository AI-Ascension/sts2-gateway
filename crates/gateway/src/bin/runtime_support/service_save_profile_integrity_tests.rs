// SPDX-License-Identifier: MIT

use super::*;
use std::fs;
use std::path::PathBuf;
use sts2_gateway::{FilesystemUserDataPort, SqliteSaveProfileOperationStore, SqliteUserDataRecordStore};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!("gateway-save-profile-http-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).map_err(|error| error.to_string())?;
        fs::create_dir(path.join("data")).map_err(|error| error.to_string())?;
        fs::write(path.join("data/existing-profile"), b"preserve").map_err(|error| error.to_string())?;
        Ok(Self(path))
    }

    fn dependencies(&self) -> Result<SaveProfileDependencies, String> {
        Ok(SaveProfileDependencies {
            intents: Some(Box::new(SqliteSaveProfileOperationStore::open(self.0.join("operations.sqlite"))
                .map_err(|error| format!("{error:?}"))?)),
            allocation_port: Some(Box::new(FilesystemUserDataPort::try_new(self.0.join("data"), 8)
                .map_err(|error| format!("{error:?}"))?)),
            allocation_intents: Some(Box::new(SqliteUserDataRecordStore::open(self.0.join("allocations.sqlite"))
                .map_err(|error| format!("{error:?}"))?)),
            // Explicit synthetic authority: no native process/host mapping is claimed.
            launch_profile: Some(Box::new(InMemoryLaunchProfileBindingPort::default())),
        })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn recording_peer(listener: TcpListener, response: Vec<u8>) -> thread::JoinHandle<Result<HttpRequest, String>> {
    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => return Err(format!("recording peer accept failed: {error}")),
            }
        };
        let request = super::super::super::http::read_request(&mut stream)
            .map_err(|status| format!("recording peer request failed: {status}"))?;
        super::super::super::http::write_response(&mut stream, 200, &response)
            .map_err(|error| error.to_string())?;
        Ok(request)
    })
}

#[test]
fn runtime_seam_checks_authority_before_physical_allocation_or_forwarding() -> Result<(), String> {
    let fixture = Fixture::new()?;
    let mut service = test_service()?;
    let (listener, address) = closed_listener()?;
    inject(&mut service, &address, Some(false), fixture.dependencies()?)?;
    let authority = SaveProfileAuthority {
        instance_id: "instance-1".to_owned(),
        caller_id: "harness".to_owned(),
        session_id: "session-1".to_owned(),
        lease_id: "lease-1".to_owned(),
        lease_epoch: 1,
        expires_at_millis: None,
    };
    for (header, value) in [
        ("x-sts2-instance-id", "foreign-instance"),
        ("x-sts2-caller-id", "foreign-caller"),
        ("x-sts2-session-id", "foreign-session"),
        ("x-sts2-lease-id", "foreign-lease"),
        ("x-sts2-lease-epoch", "2"),
    ] {
        let mut request = mutation_request("/v1/instances/instance-1/save-profile/create-disposable", "op-create", b"{}".to_vec());
        request.headers.insert(header.to_owned(), value.to_owned());
        let (status, body) = service.save_profile.request(
            super::super::super::save_profile::RuntimeSaveProfileRoute::CreateDisposable,
            &request, authority.clone(), SaveProfileActiveRun { governed: Some(false) }, 1,
        );
        assert_eq!(status, 409);
        assert_eq!(error_code(&body)?, "save_profile_fence_rejected");
    }
    assert!(!fixture.0.join("data/run-1").exists());
    assert!(matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock));
    drop(service);
    let mut store = SqliteUserDataRecordStore::open(fixture.0.join("allocations.sqlite"))
        .map_err(|error| format!("{error:?}"))?;
    assert!(sts2_gateway::UserDataRecordStore::list(&mut store).map_err(|error| format!("{error:?}"))?.is_empty());
    Ok(())
}

#[test]
fn real_allocation_and_recording_mod_receipt_reopen_without_duplicate_effects() -> Result<(), String> {
    let fixture = Fixture::new()?;
    let (listener, address) = closed_listener()?;
    let server = recording_peer(listener, create_response("op-create", 1));
    let mut service = test_service()?;
    inject(&mut service, &address, Some(false), fixture.dependencies()?)?;
    let request = mutation_request("/v1/instances/instance-1/save-profile/create-disposable", "op-create", b"{}".to_vec());
    let response = service.handle_request(&request);
    let forwarded = server.join().map_err(|_| "recording peer panicked")??;
    assert_eq!(response.0, 200, "{}", String::from_utf8_lossy(&response.1));
    assert_eq!(forwarded.path, "/api/v1/save-profile/create-disposable");
    let body: Value = serde_json::from_slice(&forwarded.body).map_err(|error| error.to_string())?;
    assert_eq!(body["user_data"]["identity"], 1);
    assert_eq!(body["launch_profile"]["user_data"], 1);
    let result: Value = serde_json::from_slice(&response.1).map_err(|error| error.to_string())?;
    assert_eq!(result["baseline"]["digest"], DIGEST);
    let allocation = fixture.0.join("data/run-1");
    let provenance = fs::read(allocation.join("gateway-provenance.json")).map_err(|error| error.to_string())?;
    drop(service);

    let (listener, address) = closed_listener()?;
    let mut reopened = test_service()?;
    inject(&mut reopened, &address, Some(false), fixture.dependencies()?)?;
    assert_eq!(reopened.handle_request(&request), response);
    assert_eq!(fs::read(allocation.join("gateway-provenance.json")).map_err(|error| error.to_string())?, provenance);
    fs::remove_file(allocation.join("gateway-provenance.json")).map_err(|error| error.to_string())?;
    fs::write(allocation.join("foreign-save"), b"preserve unknown").map_err(|error| error.to_string())?;
    let (status, body) = reopened.handle_request(&request);
    assert_eq!(status, 409);
    assert_eq!(serde_json::from_slice::<Value>(&body).map_err(|error| error.to_string())?["status"], "blocked");
    assert!(matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock));
    assert!(!fixture.0.join("data/run-2").exists());
    assert_eq!(fs::read(fixture.0.join("data/existing-profile")).map_err(|error| error.to_string())?, b"preserve");
    assert_eq!(fs::read(allocation.join("foreign-save")).map_err(|error| error.to_string())?, b"preserve unknown");
    Ok(())
}
