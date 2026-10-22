// SPDX-License-Identifier: MIT

use super::*;

pub(in crate::process_native) fn launch_arguments(
    record: &GenerationRecord,
    profile: &NativeProfile,
) -> Vec<String> {
    let user_data = profile.user_data.to_string_lossy();
    let policy = profile.profile.policy();
    let mut arguments = vec![
        "--user".to_owned(),
        "--quiet".to_owned(),
        "--no-ask-password".to_owned(),
        format!("--unit={}", record.unit),
        format!("--description={}", description(record)),
        "--service-type=exec".to_owned(),
        "--remain-after-exit".to_owned(),
        "--expand-environment=no".to_owned(),
        "--property=ExitType=cgroup".to_owned(),
        "--property=KillMode=control-group".to_owned(),
        "--property=SendSIGKILL=yes".to_owned(),
        "--property=Restart=no".to_owned(),
        "--property=Delegate=no".to_owned(),
        "--property=ProtectControlGroups=yes".to_owned(),
        "--property=NoNewPrivileges=yes".to_owned(),
        "--property=ProtectSystem=strict".to_owned(),
        "--property=ProtectHome=read-only".to_owned(),
        "--property=PrivateTmp=yes".to_owned(),
        "--property=StandardOutput=null".to_owned(),
        "--property=StandardError=null".to_owned(),
        format!("--property=TasksMax={}", policy.max_descendants() + 1),
        format!(
            "--property=TimeoutStartSec={}ms",
            policy.start_timeout_millis()
        ),
        format!(
            "--property=TimeoutStopSec={}ms",
            policy.stop_timeout_millis()
        ),
        format!("--property=ReadWritePaths={user_data}"),
        format!("--working-directory={user_data}"),
        format!("--setenv=HOME={user_data}"),
        format!("--setenv=XDG_DATA_HOME={user_data}/data"),
        format!("--setenv=XDG_CONFIG_HOME={user_data}/config"),
        format!("--setenv=XDG_CACHE_HOME={user_data}/cache"),
        "--".to_owned(),
        profile.executable.to_string_lossy().into_owned(),
    ];
    arguments.extend(profile.arguments.iter().cloned());
    arguments
}
