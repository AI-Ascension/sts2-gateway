// SPDX-License-Identifier: MIT

use super::authorization::first_rejected_header;
use super::*;

impl RuntimeService {
    pub(super) fn handle_queued_request(&mut self, queued: QueuedRequest) -> Result<(), String> {
        let QueuedRequest {
            mut stream,
            request,
            cancellation,
            cancellation_watcher,
        } = queued;
        let result = if cancellation.is_cancelled() {
            Ok(())
        } else {
            let (status, body) = self.handle_request_with_cancellation(&request, &cancellation);
            if cancellation.is_cancelled() {
                Ok(())
            } else {
                write_response(&mut stream, status, &body).map_err(|error| error.to_string())
            }
        };
        finish_cancellation(cancellation, cancellation_watcher);
        result
    }

    pub(super) fn cancel_queued_request(&self, queued: QueuedRequest) -> Result<(), String> {
        let QueuedRequest {
            mut stream,
            cancellation,
            cancellation_watcher,
            ..
        } = queued;
        let result = if cancellation.is_cancelled() {
            Ok(())
        } else {
            write_response(
                &mut stream,
                503,
                &json_error("runtime_v2_shutdown_admission_closed"),
            )
            .map_err(|error| error.to_string())
        };
        finish_cancellation(cancellation, cancellation_watcher);
        result
    }
}

const CALLER_DISCONNECT_POLL: Duration = Duration::from_millis(25);

fn spawn_caller_watcher(
    stream: &TcpStream,
    cancellation: &RequestCancellation,
) -> thread::JoinHandle<()> {
    let cancellation = cancellation.clone();
    let Ok(caller) = stream.try_clone() else {
        cancellation.cancel();
        return thread::spawn(|| {});
    };
    thread::spawn(move || {
        if caller
            .set_read_timeout(Some(CALLER_DISCONNECT_POLL))
            .is_err()
        {
            cancellation.cancel();
            return;
        }
        let mut byte = [0_u8; 1];
        while !cancellation.is_complete() {
            match caller.peek(&mut byte) {
                Ok(0) => {
                    cancellation.cancel();
                    return;
                }
                Ok(_) => thread::sleep(CALLER_DISCONNECT_POLL),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) => {}
                Err(_) => {
                    cancellation.cancel();
                    return;
                }
            }
        }
    })
}

fn finish_cancellation(
    cancellation: RequestCancellation,
    cancellation_watcher: Option<thread::JoinHandle<()>>,
) {
    cancellation.complete();
    if let Some(watcher) = cancellation_watcher {
        let _ = watcher.join();
    }
}

fn reject_queued_request(queued: QueuedRequest, status: u16, body: &[u8]) -> Result<(), String> {
    let QueuedRequest {
        mut stream,
        cancellation,
        cancellation_watcher,
        ..
    } = queued;
    let result = if cancellation.is_cancelled() {
        Ok(())
    } else {
        write_response(&mut stream, status, body).map_err(|error| error.to_string())
    };
    finish_cancellation(cancellation, cancellation_watcher);
    result
}
pub(super) fn run_worker(
    mut service: RuntimeService,
    receiver: Receiver<QueuedRequest>,
    admission_open: Arc<AtomicBool>,
    listener_address: SocketAddr,
) -> Result<(), String> {
    while let Ok(queued) = receiver.recv() {
        let service_started = Instant::now();
        service.metrics.work_started();
        let result = service.handle_queued_request(queued);
        service.metrics.work_completed(service_started.elapsed());
        if let Err(error) = result {
            eprintln!("gateway queued request failed: {error}");
        }
        if service.shutdown_requested {
            admission_open.store(false, Ordering::Release);
            // Wake admission before draining. A listener may still be parsing a
            // request; retain the receiver until that producer has exited so a
            // concurrent successful enqueue always receives cancellation.
            wake_listener(listener_address);
            while let Ok(queued) = receiver.recv() {
                service.metrics.work_cancelled_on_shutdown();
                if let Err(error) = service.cancel_queued_request(queued) {
                    eprintln!("gateway shutdown cancellation failed: {error}");
                }
            }
            return Ok(());
        }
    }
    admission_open.store(false, Ordering::Release);
    Ok(())
}

pub(super) fn accept_requests(
    listener: TcpListener,
    sender: SyncSender<QueuedRequest>,
    admission_open: Arc<AtomicBool>,
    auth_policy: AuthPolicy,
    instance_id: String,
    recovery_enabled: bool,
    metrics: RuntimeMetrics,
) -> Result<(), String> {
    loop {
        if !admission_open.load(Ordering::Acquire) {
            break;
        }
        let (mut stream, _) = listener
            .accept()
            .map_err(|error| format!("gateway accept failed: {error}"))?;
        if !admission_open.load(Ordering::Acquire) {
            break;
        }
        stream
            .set_read_timeout(Some(REQUEST_READ_TIMEOUT))
            .map_err(|error| format!("gateway request timeout setup failed: {error}"))?;
        stream
            .set_write_timeout(Some(REQUEST_WRITE_TIMEOUT))
            .map_err(|error| format!("gateway response timeout setup failed: {error}"))?;
        metrics.request_seen();
        let request = match read_request(&mut stream) {
            Ok(request) => request,
            Err(status) => {
                metrics.malformed_rejected();
                let _ = write_response(&mut stream, status, &json_error("malformed_request"));
                continue;
            }
        };
        if let Some((status, body)) =
            request_rejection(&request, &auth_policy, &instance_id, recovery_enabled)
        {
            report_rejected_header(&request, status);
            if status == 401 || status == 403 {
                metrics.authentication_rejected();
            } else {
                metrics.malformed_rejected();
            }
            let _ = write_response(&mut stream, status, &body);
            continue;
        }
        let cancellation = RequestCancellation::new();
        // Only game-information exchanges need caller cancellation propagated to
        // the producer.  Avoid one long-lived watcher thread for every unrelated
        // gateway route while retaining disconnect cancellation for this bounded
        // read surface.
        let cancellation_watcher =
            GameInformationRoute::parse(&request.method, &request.path, &instance_id)
                .is_some()
                .then(|| spawn_caller_watcher(&stream, &cancellation));
        let queued = QueuedRequest {
            stream,
            request,
            cancellation,
            cancellation_watcher,
        };
        if !admission_open.load(Ordering::Acquire) {
            let _ = reject_queued_request(
                queued,
                503,
                &json_error("runtime_v2_shutdown_admission_closed"),
            );
            break;
        }
        // Publish accounting before the receiver can consume the request.
        // Failed nonblocking sends roll back their reservation.
        metrics.queue_admitted();
        match sender.try_send(queued) {
            Ok(()) => {}
            Err(TrySendError::Full(queued)) => {
                metrics.queue_admission_reverted();
                metrics.queue_rejected();
                let _ =
                    reject_queued_request(queued, 429, &json_overload("runtime_v2_queue_capacity"));
            }
            Err(TrySendError::Disconnected(queued)) => {
                metrics.queue_admission_reverted();
                let _ = reject_queued_request(
                    queued,
                    503,
                    &json_error("runtime_v2_shutdown_admission_closed"),
                );
                break;
            }
        }
    }
    Ok(())
}

/// Records which header a refusal named, on the gateway's own diagnostic output.
///
/// The refusal body already carries the name for the caller, but a name that is
/// only written to a socket is invisible to an operator holding the run artifact
/// and no client transcript, which is exactly the untraced
/// `unsupported_header` refusal on AI-Ascension/sts2-harness#541.
///
/// Only the *name* is printed. A header value may carry a credential, so the
/// value is never read here: `first_rejected_header` yields a `&str` borrowed
/// from the header map's keys, and the format argument below is that borrow,
/// so no value byte has a path to the stream. The name is constrained to the
/// RFC 7230 token charset by `valid_header` at parse time, so it cannot carry a
/// delimiter or forge a second field. The request line, the other header names,
/// and the method and path are all withheld for the same reason.
///
/// This is `eprintln!` to match this file's existing diagnostic idiom (the
/// queued-request and shutdown-cancellation paths) rather than `log::` or
/// `tracing::`, neither of which this binary initialises. That is deliberate: an
/// uninitialised facade would silently discard the line, which is the gap this
/// closes.
fn report_rejected_header(request: &HttpRequest, status: u16) {
    let Some(name) = first_rejected_header(&request.headers) else {
        return;
    };
    if status != 400 {
        return;
    }
    eprintln!("gateway refused unsupported header: {name}");
}

pub(super) fn wake_listener(address: SocketAddr) {
    let _ = TcpStream::connect_timeout(&address, Duration::from_millis(200));
}
