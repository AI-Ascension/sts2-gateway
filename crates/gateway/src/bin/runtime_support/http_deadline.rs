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
