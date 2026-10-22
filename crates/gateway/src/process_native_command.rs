// SPDX-License-Identifier: MIT

//! Bounded command children; output is private scratch and is never used as an error message.

use super::NativeProcessError;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const MAX_OUTPUT: u64 = 65_536;

pub(super) struct CommandOutput {
    pub(super) success: bool,
    pub(super) bytes: Vec<u8>,
}

struct OutputFile(PathBuf);
impl Drop for OutputFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub(super) fn run(
    executable: &Path,
    arguments: &[String],
    runtime_directory: &Path,
    scratch: &Path,
    budget: Duration,
) -> Result<CommandOutput, NativeProcessError> {
    if budget.is_zero() {
        return Err(NativeProcessError::UnsupportedHost);
    }
    let deadline = Instant::now() + budget;
    let output_path = OutputFile(scratch.join(format!(
        "process-command-{}.out",
        uuid::Uuid::new_v4().simple()
    )));
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let output = options
        .open(&output_path.0)
        .map_err(|_| NativeProcessError::UnsupportedHost)?;
    let mut child = Command::new(executable)
        .args(arguments)
        .env_clear()
        .env("XDG_RUNTIME_DIR", runtime_directory)
        .env("LANG", "C")
        .env("SYSTEMD_COLORS", "0")
        .env("SYSTEMD_LOG_LEVEL", "err")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(
            output
                .try_clone()
                .map_err(|_| NativeProcessError::UnsupportedHost)?,
        )
        .spawn()
        .map_err(|_| NativeProcessError::UnsupportedHost)?;
    let result = wait(&mut child, &output, deadline);
    let status = match result {
        Ok(status) => status,
        Err(error) => {
            // Child owns this unreaped PID; this never targets a registry or caller PID.
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let bytes = super::configuration::bounded_read(&output_path.0, MAX_OUTPUT)?;
    Ok(CommandOutput {
        success: status.success(),
        bytes,
    })
}

fn wait(
    child: &mut Child,
    output: &File,
    deadline: Instant,
) -> Result<std::process::ExitStatus, NativeProcessError> {
    loop {
        if output
            .metadata()
            .map_err(|_| NativeProcessError::UnsupportedHost)?
            .len()
            > MAX_OUTPUT
        {
            return Err(NativeProcessError::UnsupportedHost);
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|_| NativeProcessError::UnsupportedHost)?
        {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            return Err(NativeProcessError::UnsupportedHost);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

pub(super) fn version(output: &CommandOutput) -> Result<u64, NativeProcessError> {
    if !output.success {
        return Err(NativeProcessError::UnsupportedHost);
    }
    let text =
        std::str::from_utf8(&output.bytes).map_err(|_| NativeProcessError::UnsupportedHost)?;
    let text = text.trim().strip_prefix("systemd ").unwrap_or(text.trim());
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    let value = digits
        .parse::<u64>()
        .map_err(|_| NativeProcessError::UnsupportedHost)?;
    if value < 254 {
        return Err(NativeProcessError::UnsupportedHost);
    }
    Ok(value)
}
