// SPDX-License-Identifier: MIT

use crate::identity::InstanceId;
use crate::lifecycle::InstanceRecord;
use crate::ports::{Clock, ProcessPort, StopMode};
use crate::{CleanupStatus, ExpirationEvent, Gateway, ShutdownReport};

impl<C, P, R, T, F> Gateway<C, P, R, T, F>
where
    C: Clock,
    P: ProcessPort,
{
    pub fn expire_due(&mut self) -> Vec<ExpirationEvent> {
        let now = self.clock.now();
        let expired: Vec<InstanceId> = self
            .instances
            .iter()
            .filter_map(|(instance_id, record)| {
                record
                    .lease()
                    .filter(|lease| lease.expires_at() <= now)
                    .map(|_| *instance_id)
            })
            .collect();
        expired
            .into_iter()
            .map(|instance_id| ExpirationEvent {
                instance_id,
                status: self.expire_instance(instance_id),
            })
            .collect()
    }

    pub fn shutdown(&mut self) -> ShutdownReport {
        self.admitting = false;
        let instance_ids: Vec<InstanceId> = self.instances.keys().copied().collect();
        let mut stopped = 0;
        let mut failed = 0;
        for instance_id in instance_ids {
            let Some(record) = self.instances.get(&instance_id) else {
                continue;
            };
            let process = record.process();
            let state = record.state();
            if state.is_terminal() && process.is_none() {
                continue;
            }
            match process {
                Some(process) => match self.process.stop(process, StopMode::Graceful) {
                    Ok(()) => {
                        if let Some(record) = self.instances.get_mut(&instance_id) {
                            record.clear_process();
                            record.mark_stopped();
                        }
                        stopped += 1;
                    }
                    Err(_) => {
                        if let Some(record) = self.instances.get_mut(&instance_id) {
                            record.mark_failed();
                        }
                        failed += 1;
                    }
                },
                None => {
                    if let Some(record) = self.instances.get_mut(&instance_id) {
                        record.mark_stopped();
                    }
                    stopped += 1;
                }
            }
        }
        ShutdownReport { stopped, failed }
    }

    pub(crate) fn lease_expired(&self, instance_id: InstanceId) -> bool {
        self.instances
            .get(&instance_id)
            .and_then(InstanceRecord::lease)
            .is_some_and(|lease| lease.expires_at() <= self.clock.now())
    }

    pub(crate) fn expire_instance(&mut self, instance_id: InstanceId) -> CleanupStatus {
        let process = self
            .instances
            .get(&instance_id)
            .and_then(InstanceRecord::process);
        match process {
            Some(process) => match self.process.stop(process, StopMode::Force) {
                Ok(()) => {
                    if let Some(record) = self.instances.get_mut(&instance_id) {
                        record.clear_process();
                        record.mark_expired();
                    }
                    CleanupStatus::Cleaned
                }
                Err(fault) => {
                    if let Some(record) = self.instances.get_mut(&instance_id) {
                        record.mark_failed();
                    }
                    CleanupStatus::Failed(fault)
                }
            },
            None => {
                if let Some(record) = self.instances.get_mut(&instance_id) {
                    record.mark_expired();
                }
                CleanupStatus::Cleaned
            }
        }
    }
}
