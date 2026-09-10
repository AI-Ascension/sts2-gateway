// SPDX-License-Identifier: MIT

use super::{HttpRequest, MAX_RESPONSE_BYTES, RuntimeService, json_error};

#[path = "service_receipt_query_validation.rs"]
mod validation;
pub(super) use validation::{ReceiptQueryValidationError, validate_request, validate_response};

const DOWNSTREAM_PATH: &str = "/api/v1/coop/native/receipt-query";

impl RuntimeService {
    pub(super) fn coop_receipt_query(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if !request.content_type_is_json() {
            return (400, json_error("coop_receipt_query_content_type_required"));
        }

        let request_value = match validate_request(&request.body, &request.headers) {
            Ok(value) => value,
            Err(error) => {
                return (
                    request_error_status(error),
                    json_error(request_error_code(error)),
                );
            }
        };
        let correlation = request
            .headers
            .get("x-sts2-correlation-id")
            .map(String::as_str);
        let response = match self.forward_mod_with_limit(
            "POST",
            DOWNSTREAM_PATH,
            &request.body,
            correlation,
            MAX_RESPONSE_BYTES,
        ) {
            Ok(response) => response,
            Err(status) => {
                return (
                    status,
                    json_error("coop_receipt_query_downstream_unavailable"),
                );
            }
        };
        if let Err(error) = validate_response(&request_value, response.status, &response.body) {
            return (502, json_error(response_error_code(error)));
        }
        (response.status, response.body)
    }

    pub(super) fn coop_receipt_query_path(&self) -> String {
        format!(
            "/v1/instances/{}/coop/receipt-query",
            self.config.instance_id
        )
    }
}
fn request_error_status(error: ReceiptQueryValidationError) -> u16 {
    match error {
        ReceiptQueryValidationError::RequestBodyRequired
        | ReceiptQueryValidationError::RequestBodyInvalid => 400,
        ReceiptQueryValidationError::RequestBodyOversized => 413,
        ReceiptQueryValidationError::ResponseBodyOversized
        | ReceiptQueryValidationError::ResponseBodyInvalid => 502,
    }
}

fn request_error_code(error: ReceiptQueryValidationError) -> &'static str {
    match error {
        ReceiptQueryValidationError::RequestBodyRequired => "coop_receipt_query_body_required",
        ReceiptQueryValidationError::RequestBodyOversized => "coop_receipt_query_body_oversized",
        ReceiptQueryValidationError::RequestBodyInvalid => "coop_receipt_query_request_invalid",
        ReceiptQueryValidationError::ResponseBodyOversized => {
            "coop_receipt_query_response_oversized"
        }
        ReceiptQueryValidationError::ResponseBodyInvalid => "coop_receipt_query_response_invalid",
    }
}

fn response_error_code(error: ReceiptQueryValidationError) -> &'static str {
    match error {
        ReceiptQueryValidationError::ResponseBodyOversized => {
            "coop_receipt_query_response_oversized"
        }
        ReceiptQueryValidationError::ResponseBodyInvalid => "coop_receipt_query_response_invalid",
        _ => "coop_receipt_query_response_invalid",
    }
}
