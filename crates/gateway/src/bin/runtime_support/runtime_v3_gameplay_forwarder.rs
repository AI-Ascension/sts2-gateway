// SPDX-License-Identifier: MIT

use serde_json::Value;

use super::runtime_v3_gameplay::RuntimeV3GameplayRoute;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeV3GameplayForwarder {
    max_request_bytes: usize,
    max_response_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeV3GameplayForwardError {
    RequestBodyRequired,
    RequestBodyOversized,
    RequestBodyMalformed,
    ResponseOversized,
    ResponseMalformed,
}

impl RuntimeV3GameplayForwarder {
    pub(crate) const fn new(max_request_bytes: usize, max_response_bytes: usize) -> Self {
        Self {
            max_request_bytes,
            max_response_bytes,
        }
    }

    pub(crate) fn validate_request(
        self,
        _route: RuntimeV3GameplayRoute,
        body: &[u8],
    ) -> Result<(), RuntimeV3GameplayForwardError> {
        if body.len() > self.max_request_bytes {
            return Err(RuntimeV3GameplayForwardError::RequestBodyOversized);
        }
        if body.is_empty() {
            return Err(RuntimeV3GameplayForwardError::RequestBodyRequired);
        }
        let value = serde_json::from_slice::<Value>(body)
            .map_err(|_| RuntimeV3GameplayForwardError::RequestBodyMalformed)?;
        if !value.is_object() {
            return Err(RuntimeV3GameplayForwardError::RequestBodyMalformed);
        }
        Ok(())
    }

    pub(crate) fn validate_response(
        self,
        body: &[u8],
    ) -> Result<(), RuntimeV3GameplayForwardError> {
        if body.len() > self.max_response_bytes {
            return Err(RuntimeV3GameplayForwardError::ResponseOversized);
        }
        let value = serde_json::from_slice::<Value>(body)
            .map_err(|_| RuntimeV3GameplayForwardError::ResponseMalformed)?;
        if !value.is_object() {
            return Err(RuntimeV3GameplayForwardError::ResponseMalformed);
        }
        Ok(())
    }
}
