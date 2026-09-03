// SPDX-License-Identifier: MIT

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use serde_json::{Value, json};

#[derive(Clone, Default)]
pub(crate) struct RuntimeMetrics {
    inner: Arc<RuntimeMetricsInner>,
}

#[derive(Default)]
struct RuntimeMetricsInner {
    requests_seen: AtomicU64,
    requests_admitted: AtomicU64,
    queue_rejected: AtomicU64,
    authentication_rejected: AtomicU64,
    malformed_rejected: AtomicU64,
    completed: AtomicU64,
    cancelled_on_shutdown: AtomicU64,
    active: AtomicUsize,
    queued: AtomicUsize,
    shutdown_requested: AtomicBool,
}

impl RuntimeMetrics {
    pub(crate) fn request_seen(&self) {
        self.inner.requests_seen.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn authentication_rejected(&self) {
        self.inner
            .authentication_rejected
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn malformed_rejected(&self) {
        self.inner
            .malformed_rejected
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn queue_admitted(&self) {
        self.inner.requests_admitted.fetch_add(1, Ordering::Relaxed);
        self.inner.queued.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn queue_rejected(&self) {
        self.inner.queue_rejected.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn work_started(&self) {
        self.inner.queued.fetch_sub(1, Ordering::Relaxed);
        self.inner.active.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn work_completed(&self) {
        self.inner.active.fetch_sub(1, Ordering::Relaxed);
        self.inner.completed.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn work_cancelled_on_shutdown(&self) {
        self.inner.queued.fetch_sub(1, Ordering::Relaxed);
        self.inner
            .cancelled_on_shutdown
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn request_shutdown(&self) {
        self.inner.shutdown_requested.store(true, Ordering::Release);
    }

    pub(crate) fn is_shutdown_requested(&self) -> bool {
        self.inner.shutdown_requested.load(Ordering::Acquire)
    }

    pub(crate) fn snapshot(&self, instance_id: &str, queue_capacity: usize) -> Value {
        json!({
            "instance_id": instance_id,
            "queue_capacity": queue_capacity,
            "queue_depth": self.inner.queued.load(Ordering::Relaxed),
            "active_requests": self.inner.active.load(Ordering::Relaxed),
            "requests_seen": self.inner.requests_seen.load(Ordering::Relaxed),
            "requests_admitted": self.inner.requests_admitted.load(Ordering::Relaxed),
            "queue_rejected": self.inner.queue_rejected.load(Ordering::Relaxed),
            "authentication_rejected": self.inner.authentication_rejected.load(Ordering::Relaxed),
            "malformed_rejected": self.inner.malformed_rejected.load(Ordering::Relaxed),
            "completed": self.inner.completed.load(Ordering::Relaxed),
            "cancelled_on_shutdown": self.inner.cancelled_on_shutdown.load(Ordering::Relaxed),
            "shutdown_requested": self.is_shutdown_requested(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::RuntimeMetrics;

    #[test]
    fn snapshot_tracks_queue_and_shutdown_transitions() {
        let metrics = RuntimeMetrics::default();
        metrics.request_seen();
        metrics.queue_admitted();
        metrics.work_started();
        metrics.work_completed();
        metrics.queue_admitted();
        metrics.work_cancelled_on_shutdown();
        metrics.request_shutdown();

        let snapshot = metrics.snapshot("instance-1", 2);
        assert_eq!(snapshot["queue_capacity"], 2);
        assert_eq!(snapshot["queue_depth"], 0);
        assert_eq!(snapshot["active_requests"], 0);
        assert_eq!(snapshot["requests_seen"], 1);
        assert_eq!(snapshot["requests_admitted"], 2);
        assert_eq!(snapshot["completed"], 1);
        assert_eq!(snapshot["cancelled_on_shutdown"], 1);
        assert_eq!(snapshot["shutdown_requested"], true);
    }
}
