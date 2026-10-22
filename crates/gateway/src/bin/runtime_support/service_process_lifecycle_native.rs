// SPDX-License-Identifier: MIT

use super::*;

/// Composes the lifecycle surface from configuration.
///
/// Legacy configuration remains unavailable. Explicit ADR 0037 bindings opt in to the
/// gateway-owned systemd adapter only after complete profile and host validation.
pub(super) fn from_environment(
    config: &RuntimeConfig,
) -> Result<service_process_lifecycle::ProcessLifecycleRuntime, String> {
    let deployment = service_process_lifecycle_config::from_environment()?;
    let native_path = std::env::var_os("STS2_PROCESS_LIFECYCLE_NATIVE_CONFIG");
    from_configuration(
        deployment,
        native_path.as_deref().map(std::path::Path::new),
        &[
            config.journal_path.as_deref(),
            config.recovery_store_path.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>(),
    )
}

pub(in crate::runtime_support::service) fn from_configuration(
    deployment: Option<service_process_lifecycle_config::ProcessLifecycleDeployment>,
    native_path: Option<&std::path::Path>,
    protected_paths: &[&std::path::Path],
) -> Result<service_process_lifecycle::ProcessLifecycleRuntime, String> {
    match (deployment, native_path) {
        (Some(deployment), None) => {
            service_process_lifecycle::ProcessLifecycleRuntime::configured(&deployment)
        }
        (Some(deployment), Some(path)) => {
            let profiles = deployment
                .profiles
                .iter()
                .map(|entry| entry.to_profile())
                .collect::<Result<Vec<_>, _>>()?;
            let mut protected_paths = protected_paths.to_vec();
            protected_paths.push(&deployment.store_path);
            let process = sts2_gateway::SystemdProcessPort::open(
                path,
                &profiles,
                deployment.max_records,
                &protected_paths,
            )
            .map_err(|error| format!("native process adapter refused startup: {error:?}"))?;
            service_process_lifecycle::ProcessLifecycleRuntime::compose(
                deployment,
                Box::new(process),
            )
        }
        (None, Some(_)) => {
            Err("native process configuration requires the lifecycle deployment".to_owned())
        }
        (None, None) => Ok(service_process_lifecycle::ProcessLifecycleRuntime::unconfigured()),
    }
}
