// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn production_dispatch_forwards_each_fixed_phase_with_live_owner_and_correlation()
-> Result<(), String> {
    let mut ready = ready_service()?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .to_string();
    ready.service.config.mod_address = address;
    let responder = std::thread::spawn(move || -> Result<Vec<(HttpRequest, String)>, String> {
        let mut received = Vec::new();
        for (route, kind) in [
            (Route::Begin, "exact_restore_begin_request"),
            (Route::Chunk, "exact_restore_chunk_request"),
            (Route::Finish, "exact_restore_finish_blob_request"),
            (Route::Commit, "exact_restore_commit_request"),
            (Route::Lookup, "exact_restore_lookup_request"),
        ] {
            let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
            let downstream = read_request(&mut stream).map_err(|error| format!("{error:?}"))?;
            if downstream.method != "POST" || downstream.path != route.path() {
                return Err(format!(
                    "unexpected downstream route: {} {}",
                    downstream.method, downstream.path
                ));
            }
            let frame: Value =
                serde_json::from_slice(&downstream.body).map_err(|error| error.to_string())?;
            if frame["kind"] != kind {
                return Err(format!("unexpected neutral phase: {frame:?}"));
            }
            let bytes = error_response(&frame, "UNAVAILABLE", "native_unavailable")?;
            write_response(&mut stream, 503, &bytes).map_err(|error| error.to_string())?;
            received.push((
                downstream,
                frame["correlation_id"]
                    .as_str()
                    .ok_or("missing forwarded correlation")?
                    .to_owned(),
            ));
        }
        Ok(received)
    });

    let phases = [
        (Route::Begin, "exact_restore_begin_request"),
        (Route::Chunk, "exact_restore_chunk_request"),
        (Route::Finish, "exact_restore_finish_blob_request"),
        (Route::Commit, "exact_restore_commit_request"),
        (Route::Lookup, "exact_restore_lookup_request"),
    ];
    for (index, (route, kind)) in phases.into_iter().enumerate() {
        let message_id = Uuid::new_v4().to_string();
        let correlation_id = Uuid::new_v4().to_string();
        let frame = request_frame(&ready.owner, kind, &message_id, &correlation_id)?;
        let body = wrapper_request(frame, &ready.service.config.caller_id)?;
        assert!(body.len() <= MAX_FRAME_BYTES);
        let request =
            authenticated_exact_request(&ready.service, route.path(), body, &correlation_id);
        let (status, body) = ready.service.handle_request(&request);
        assert_eq!(status, 503, "phase {index}");
        let response: Value = serde_json::from_slice(&body).map_err(|error| error.to_string())?;
        assert_eq!(response["kind"], "exact_restore_response");
        assert_eq!(response["actor"]["role"], "gateway");
        assert_eq!(response["auth"]["capability"], "exact_restore");
        assert_eq!(response["payload"]["frame"]["correlation_id"], message_id);
        assert_eq!(
            response["payload"]["frame"]["payload"]["expected_owner"],
            ready.owner
        );
    }
    let received = responder
        .join()
        .map_err(|_| "native test peer panicked".to_owned())??;
    assert_eq!(received.len(), phases.len());
    for (downstream, correlation_id) in received {
        assert_eq!(
            downstream.headers.get("x-sts2-correlation-id"),
            Some(&correlation_id)
        );
        assert_eq!(
            downstream
                .headers
                .get("x-sts2-instance-id")
                .map(String::as_str),
            ready.owner["instance_id"].as_str()
        );
        assert_eq!(
            downstream
                .headers
                .get("x-sts2-lease-id")
                .map(String::as_str),
            ready.owner["lease_id"].as_str()
        );
    }
    cleanup(&ready.store_path);
    Ok(())
}
