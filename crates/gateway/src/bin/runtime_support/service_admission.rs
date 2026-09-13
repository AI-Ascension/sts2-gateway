// SPDX-License-Identifier: MIT

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

pub(super) fn wake_listener(address: SocketAddr) {
    let _ = TcpStream::connect_timeout(&address, Duration::from_millis(200));
}
