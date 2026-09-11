// SPDX-License-Identifier: MIT

use super::*;

pub(crate) fn build_runtime_v2(
    config: &RuntimeConfig,
    binding: RuntimeV2Binding,
    forwarder: HttpRuntimeV2Forwarder,
) -> Result<RuntimeV2Ledger<HttpRuntimeV2Forwarder>, String> {
    let ledger = match config.workflow_authority.clone() {
        Some(authority) => {
            let contract = RuntimeV2RecoveryContract::new(
                authority,
                RuntimeV2RecoveryCapabilities::unsupported(),
            )
            .map_err(|error| format!("Runtime-v2 recovery contract is invalid: {error}"))?;
            RuntimeV2Ledger::new_with_recovery_contract(
                RuntimeV2LedgerConfig::new(config.operation_capacity),
                contract,
                RuntimeV2Observation::new(RuntimeV2CombatPhase::OutsideCombat, 0, false, 0),
                forwarder,
            )
        }
        None => RuntimeV2Ledger::new(
            RuntimeV2LedgerConfig::new(config.operation_capacity),
            binding,
            forwarder,
        ),
    };
    ledger.map_err(|error| format!("Runtime-v2 ledger is invalid: {error}"))
}
