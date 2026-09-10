// SPDX-License-Identifier: MIT

use super::*;

impl RuntimeService {
    pub(super) fn workflow_authority(
        &self,
        request: &HttpRequest,
    ) -> Result<Option<RuntimeV2Authority>, (u16, Vec<u8>)> {
        let Some(expected) = self.runtime_v2.binding().authority() else {
            return Ok(None);
        };
        let Some(boot_epoch) = request.headers.get("x-sts2-workflow-boot-epoch") else {
            return Err((409, json_error("runtime_v2_workflow_authority_required")));
        };
        let presented = RuntimeV2Authority::new(
            &self.config.instance_id,
            &self.config.session_id,
            &self.config.lease_id,
            self.config.lease_epoch,
            boot_epoch,
        )
        .map_err(|_| (400, json_error("runtime_v2_workflow_authority_invalid")))?;
        if &presented != expected {
            return Err((409, json_error("runtime_v2_workflow_authority_rejected")));
        }
        Ok(Some(presented))
    }
}
