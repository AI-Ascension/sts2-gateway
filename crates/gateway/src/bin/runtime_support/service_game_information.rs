// SPDX-License-Identifier: MIT

use serde_json::Value;

use super::super::http::ReadError;
use super::game_information::GameInformationRoute;
use super::game_information_forwarder::{
    GameInformationRequestError, GameInformationResponseError,
};
use super::game_information_payload::{MAX_CURSOR_BYTES, cursor, normalized_query};
use super::{HttpRequest, RuntimeService, json_error};

const MAX_CURSOR_BINDINGS: usize = 64;

impl RuntimeService {
    pub(super) fn game_information_request(
        &mut self,
        request: &HttpRequest,
        route: GameInformationRoute,
    ) -> (u16, Vec<u8>) {
        if let Err(error) = self.check_lease(request) {
            return error;
        }
        if route == GameInformationRoute::Capabilities {
            return self.game_information_capabilities(request);
        }
        if !request.content_type_is_json() {
            return (400, json_error("game_information_content_type_required"));
        }
        let query =
            match self
                .game_information
                .validate_request(route, &request.body, &request.headers)
            {
                Ok(value) => value,
                Err(error) => return game_information_request_error(error),
            };
        if !self.cursor_is_compatible(&query) {
            return (409, json_error("game_information_stale_cursor"));
        }
        let correlation = request
            .headers
            .get("x-sts2-correlation-id")
            .map(String::as_str);
        let response = match self.forward_mod_with_limit_detailed_timeout(
            "POST",
            route.downstream_path(),
            &request.body,
            correlation,
            super::game_information_forwarder::MAX_RESPONSE_BYTES,
            self.game_information_exchange_timeout,
        ) {
            Ok(response) => response,
            Err(error) => return game_information_transport_error(error),
        };
        let validated = match self.game_information.validate_response(
            route,
            Some(&query),
            &request.headers,
            response.status,
            &response.body,
        ) {
            Ok(response) => response,
            Err(error) => return game_information_response_error(error),
        };
        if validated.producer_error {
            return (response.status, response.body);
        }
        self.remember_next_cursor(&query, &validated.value);
        (response.status, response.body)
    }

    fn game_information_capabilities(&mut self, request: &HttpRequest) -> (u16, Vec<u8>) {
        if !request.body.is_empty() {
            return (
                400,
                json_error("game_information_capabilities_body_forbidden"),
            );
        }
        let correlation = request
            .headers
            .get("x-sts2-correlation-id")
            .map(String::as_str);
        let response = match self.forward_mod_with_limit_detailed_timeout(
            "GET",
            GameInformationRoute::Capabilities.downstream_path(),
            &[],
            correlation,
            super::game_information_forwarder::MAX_RESPONSE_BYTES,
            self.game_information_exchange_timeout,
        ) {
            Ok(response) => response,
            Err(ReadError::Oversized) => {
                return game_information_response_error(GameInformationResponseError::Oversized);
            }
            Err(ReadError::Timeout) => {
                return (504, json_error("game_information_capabilities_timeout"));
            }
            Err(ReadError::Malformed) => {
                return game_information_response_error(GameInformationResponseError::Invalid);
            }
            Err(ReadError::Unavailable) => {
                return (503, json_error("game_information_capabilities_unavailable"));
            }
        };
        let validated = match self.game_information.validate_response(
            GameInformationRoute::Capabilities,
            None,
            &request.headers,
            response.status,
            &response.body,
        ) {
            Ok(response) => response,
            Err(error) => return game_information_response_error(error),
        };
        if validated.producer_error {
            return (response.status, response.body);
        }
        (response.status, response.body)
    }

    fn cursor_is_compatible(&self, request: &Value) -> bool {
        let Some(query) = request.get("query") else {
            return false;
        };
        let Some(cursor) = cursor(query) else {
            return true;
        };
        if cursor.len() > MAX_CURSOR_BYTES {
            return false;
        }
        self.game_information_cursor_bindings
            .get(cursor)
            .is_none_or(|binding| normalized_query(query).is_some_and(|query| query == *binding))
    }

    fn remember_next_cursor(&mut self, request: &Value, response: &Value) {
        let Some(query) = request.get("query") else {
            return;
        };
        let Some(next_cursor) = response
            .get("result")
            .and_then(|result| result.get("page"))
            .and_then(|page| page.get("next_cursor"))
            .and_then(Value::as_str)
        else {
            return;
        };
        if next_cursor.len() > MAX_CURSOR_BYTES {
            return;
        }
        let Some(binding) = normalized_query(query) else {
            return;
        };
        if self.game_information_cursor_bindings.len() >= MAX_CURSOR_BINDINGS {
            self.game_information_cursor_bindings.pop_first();
        }
        self.game_information_cursor_bindings
            .insert(next_cursor.to_owned(), binding);
    }
}

fn game_information_request_error(error: GameInformationRequestError) -> (u16, Vec<u8>) {
    match error {
        GameInformationRequestError::Required => {
            (400, json_error("game_information_body_required"))
        }
        GameInformationRequestError::Oversized => {
            (413, json_error("game_information_request_oversized"))
        }
        GameInformationRequestError::Invalid => {
            (400, json_error("game_information_request_invalid"))
        }
        GameInformationRequestError::Scope => (409, json_error("game_information_scope_rejected")),
        GameInformationRequestError::Limit => (413, json_error("game_information_limits_rejected")),
    }
}

fn game_information_response_error(error: GameInformationResponseError) -> (u16, Vec<u8>) {
    match error {
        GameInformationResponseError::Oversized => {
            (502, json_error("game_information_response_oversized"))
        }
        GameInformationResponseError::Invalid => {
            (502, json_error("game_information_response_invalid"))
        }
    }
}

fn game_information_transport_error(error: ReadError) -> (u16, Vec<u8>) {
    match error {
        ReadError::Oversized => {
            game_information_response_error(GameInformationResponseError::Oversized)
        }
        ReadError::Malformed => {
            game_information_response_error(GameInformationResponseError::Invalid)
        }
        ReadError::Timeout => (504, json_error("game_information_downstream_timeout")),
        ReadError::Unavailable => (503, json_error("game_information_downstream_unavailable")),
    }
}
