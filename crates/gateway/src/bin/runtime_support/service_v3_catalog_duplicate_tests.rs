// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn durable_duplicate_replays_before_missing_catalog_admission() -> Result<(), String> {
    let (mut service, lease, path) = recovery_service()?;
    let dispatch = dispatch_envelope(&service, &lease, "durable-duplicate-correlation")?;
    capture_old_catalog(&mut service, &lease, &dispatch)?;
    let dispatch_request = runtime_request(&service, &lease, "action", dispatch.clone())?;
    let (status, body) = service.handle_request(&dispatch_request);
    assert_eq!(status, 503);
    assert_eq!(json_body(&body)?["payload"]["result"]["status"], "UNKNOWN");

    // The first dispatch consumed the catalog before the host outcome became
    // UNKNOWN. A delayed legal-actions response for that same boundary is
    // not a fresh authoritative read and must remain rejected.
    assert!(capture_old_catalog(&mut service, &lease, &dispatch).is_err());

    // A same-generation state read followed by a legal-actions read is fresh
    // catalog evidence, but it does not settle the durable UNKNOWN operation.
    refresh_same_generation_catalog(&mut service, &lease, &dispatch, "unknown-refresh")?;

    // The durable dispatch marker makes the old catalog unsafe even when the
    // host proof is unavailable. A different operation cannot be admitted,
    // while the exact original operation remains replayable.
    let mut new_dispatch = dispatch;
    new_dispatch["operation_id"] = "00000000-0000-4000-8000-000000000008".into();
    new_dispatch["correlation_id"] = "new-correlation".into();
    let new_request = runtime_request(&service, &lease, "action", new_dispatch)?;
    let (status, body) = service.handle_request(&new_request);
    assert_eq!(status, 413);
    assert_eq!(json_body(&body)?["error_code"], "recovery_bounds_exceeded");

    let (status, body) = service.handle_request(&dispatch_request);
    assert_eq!(status, 503);
    let replay = json_body(&body)?;
    assert_eq!(replay["payload"]["result"]["status"], "UNKNOWN");
    assert_eq!(
        replay["payload"]["operation"]["operation_id"],
        DISPATCH_OPERATION
    );
    cleanup(service, &path);
    Ok(())
}
