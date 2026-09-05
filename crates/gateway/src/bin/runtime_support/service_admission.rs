// SPDX-License-Identifier: MIT

use super::*;

impl RuntimeService {
    pub(super) fn handle_queued_request(
        &mut self,
        mut stream: TcpStream,
        request: HttpRequest,
    ) -> Result<(), String> {
        let (status, body) = self.handle_request(&request);
        write_response(&mut stream, status, &body).map_err(|error| error.to_string())
    }

    pub(super) fn cancel_queued_request(&self, mut stream: TcpStream) -> Result<(), String> {
        write_response(
            &mut stream,
            503,
            &json_error("runtime_v2_shutdown_admission_closed"),
        )
        .map_err(|error| error.to_string())
    }
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
        let result = service.handle_queued_request(queued.stream, queued.request);
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
                if let Err(error) = service.cancel_queued_request(queued.stream) {
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
        if let Some((status, body)) = request_rejection(&request, &auth_policy, &instance_id) {
            if status == 401 || status == 403 {
                metrics.authentication_rejected();
            } else {
                metrics.malformed_rejected();
            }
            let _ = write_response(&mut stream, status, &body);
            continue;
        }
        let queued = QueuedRequest { stream, request };
        if !admission_open.load(Ordering::Acquire) {
            let mut stream = queued.stream;
            let _ = write_response(
                &mut stream,
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
                let mut stream = queued.stream;
                let _ = write_response(
                    &mut stream,
                    429,
                    &json_overload("runtime_v2_queue_capacity"),
                );
            }
            Err(TrySendError::Disconnected(queued)) => {
                metrics.queue_admission_reverted();
                let mut stream = queued.stream;
                let _ = write_response(
                    &mut stream,
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
