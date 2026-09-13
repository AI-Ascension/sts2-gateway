// SPDX-License-Identifier: MIT

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

pub(super) const MESSAGE_TIMEOUT: Duration = Duration::from_secs(5);

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "HTTP deadline expired"))
}

pub(super) fn read(
    stream: &mut TcpStream,
    bytes: &mut [u8],
    deadline: Instant,
) -> io::Result<usize> {
    loop {
        stream.set_read_timeout(Some(remaining(deadline)?))?;
        match stream.read(bytes) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => {
                remaining(deadline)?;
                return result;
            }
        }
    }
}

/// Read with a bounded socket timeout without applying an overall deadline
/// check after a successful read.  The caller owns the overall deadline so
/// bytes returned at the timeout boundary are not discarded.
pub(super) fn read_with_timeout(
    stream: &mut TcpStream,
    bytes: &mut [u8],
    timeout: Duration,
) -> io::Result<usize> {
    stream.set_read_timeout(Some(timeout))?;
    loop {
        match stream.read(bytes) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => return result,
        }
    }
}

pub(super) fn write(stream: &mut TcpStream, mut bytes: &[u8], deadline: Instant) -> io::Result<()> {
    while !bytes.is_empty() {
        stream.set_write_timeout(Some(remaining(deadline)?))?;
        match stream.write(bytes) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(count) => bytes = &bytes[count..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    remaining(deadline).map(|_| ())
}

pub(super) fn write_with_cancellation<F>(
    stream: &mut TcpStream,
    mut bytes: &[u8],
    deadline: Instant,
    should_cancel: F,
) -> io::Result<()>
where
    F: Fn() -> bool,
{
    const CANCELLATION_POLL: Duration = Duration::from_millis(25);

    while !bytes.is_empty() {
        if should_cancel() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "HTTP exchange cancelled",
            ));
        }
        let poll_deadline = (Instant::now() + CANCELLATION_POLL).min(deadline);
        stream.set_write_timeout(Some(remaining(poll_deadline)?))?;
        match stream.write(bytes) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(_) if should_cancel() => {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "HTTP exchange cancelled",
                ));
            }
            Ok(count) => bytes = &bytes[count..],
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) && poll_deadline < deadline =>
            {
                continue;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    if should_cancel() {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "HTTP exchange cancelled",
        ));
    }
    remaining(deadline).map(|_| ())
}
