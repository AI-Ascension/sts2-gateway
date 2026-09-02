// SPDX-License-Identifier: MIT

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    match arguments().and_then(|(root, strict)| {
        repo_policy::check(&root, strict).map(|outcome| (outcome, strict))
    }) {
        Ok((outcome, strict)) => {
            for diagnostic in &outcome.diagnostics {
                println!("{diagnostic}");
            }
            println!(
                "Policy check: {} sized files, {} warning(s), {} error(s)",
                outcome.checked_files, outcome.warnings, outcome.errors
            );
            if outcome.passed(strict) {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("ERROR CFG001 policy.toml: {error}");
            ExitCode::FAILURE
        }
    }
}

fn arguments() -> Result<(PathBuf, bool), String> {
    let mut root =
        env::current_dir().map_err(|error| format!("cannot get current directory: {error}"))?;
    let mut strict = false;
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--strict" => strict = true,
            "--root" => {
                root = arguments
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--root requires a path".to_owned())?;
            }
            _ => return Err(format!("unknown argument: {argument}")),
        }
    }
    Ok((root, strict))
}
