// SPDX-License-Identifier: MIT

use super::game_information::GameInformationRoute;
use super::game_information_forwarder::{
    GameInformationCapabilities, GameInformationCapabilityAdmission,
};
use serde_json::Value;

impl GameInformationCapabilities {
    pub(crate) fn admit(
        &self,
        route: GameInformationRoute,
        request: &Value,
        body_len: usize,
    ) -> GameInformationCapabilityAdmission {
        let Some(kind) = route.query_kind() else {
            return GameInformationCapabilityAdmission::Unsupported;
        };
        if !self.query_kinds.iter().any(|advertised| advertised == kind) {
            return GameInformationCapabilityAdmission::Unsupported;
        }
        let Some(query) = request.get("query") else {
            return GameInformationCapabilityAdmission::Unsupported;
        };
        let Some(entity_kind) = query.get("entity_kind").and_then(Value::as_str) else {
            return GameInformationCapabilityAdmission::Unsupported;
        };
        if !self
            .entity_kinds
            .iter()
            .any(|advertised| advertised == entity_kind)
        {
            return GameInformationCapabilityAdmission::Unsupported;
        }
        let Some(projection) = query.get("projection").and_then(Value::as_str) else {
            return GameInformationCapabilityAdmission::Unsupported;
        };
        if !self
            .projections
            .iter()
            .any(|advertised| advertised == projection)
        {
            return GameInformationCapabilityAdmission::Unsupported;
        }
        let Some(detail_level) = query.get("detail_level").and_then(Value::as_str) else {
            return GameInformationCapabilityAdmission::Unsupported;
        };
        if !self
            .detail_levels
            .iter()
            .any(|advertised| advertised == detail_level)
        {
            return GameInformationCapabilityAdmission::Unsupported;
        }
        let Some(fields) = query.get("fields").and_then(Value::as_array) else {
            return GameInformationCapabilityAdmission::Unsupported;
        };
        if fields.iter().any(|field| {
            field
                .as_str()
                .is_none_or(|field| !self.fields.iter().any(|advertised| advertised == field))
        }) {
            return GameInformationCapabilityAdmission::Unsupported;
        }
        let mode = query
            .get("binding")
            .and_then(|binding| binding.get("mode"))
            .and_then(Value::as_str);
        if !matches!(mode, Some("static") | Some("live"))
            || (mode == Some("live") && !self.supports_live)
        {
            return GameInformationCapabilityAdmission::Unsupported;
        }
        if body_len > self.max_message_bytes
            || query
                .get("cursor")
                .and_then(Value::as_str)
                .is_some_and(|value| value.len() > self.max_cursor_bytes)
        {
            return GameInformationCapabilityAdmission::Limit;
        }
        let Some(limits) = query.get("limits").and_then(Value::as_object) else {
            return GameInformationCapabilityAdmission::Unsupported;
        };
        let within_limits = limits
            .get("page_items")
            .and_then(Value::as_u64)
            .is_some_and(|value| value <= self.limits.page_items)
            && limits
                .get("item_bytes")
                .and_then(Value::as_u64)
                .is_some_and(|value| value <= self.limits.item_bytes)
            && limits
                .get("page_bytes")
                .and_then(Value::as_u64)
                .is_some_and(|value| value <= self.limits.page_bytes)
            && limits
                .get("text_bytes")
                .and_then(Value::as_u64)
                .is_some_and(|value| value <= self.limits.text_bytes);
        if within_limits {
            GameInformationCapabilityAdmission::Allowed
        } else {
            GameInformationCapabilityAdmission::Limit
        }
    }

    pub(crate) fn response_within_budget(&self, response: &Value, body_len: usize) -> bool {
        body_len <= self.max_message_bytes
            && response
                .get("result")
                .and_then(|result| result.get("page"))
                .and_then(|page| page.get("next_cursor"))
                .and_then(Value::as_str)
                .is_none_or(|cursor| cursor.len() <= self.max_cursor_bytes)
    }
}
