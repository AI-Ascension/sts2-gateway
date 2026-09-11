// SPDX-License-Identifier: MIT

use super::{RuntimeService, json_error};

impl RuntimeService {
    pub(super) fn validate_coop_native_return_local_peer(
        &self,
        response: &[u8],
    ) -> Result<(), (u16, Vec<u8>)> {
        let Some(binding) = self.coop_native_peer_binding.as_ref() else {
            return Err((503, json_error("coop_native_peer_binding_unconfigured")));
        };
        let value = super::super::strict_json::parse(response)
            .map_err(|_| (502, json_error("coop_native_response_invalid")))?;
        let Some(peers) = value["observation"]["peers"].as_array() else {
            return Err((409, json_error("coop_native_local_peer_mismatch")));
        };
        let mut local_peers = peers
            .iter()
            .filter(|peer| peer["role"].as_str() == Some("local"));
        let local_peer = local_peers
            .next()
            .and_then(|peer| peer["peer_token"].as_str());
        if local_peer != Some(&binding.peer_id) || local_peers.next().is_some() {
            return Err((409, json_error("coop_native_local_peer_mismatch")));
        }
        Ok(())
    }
}
