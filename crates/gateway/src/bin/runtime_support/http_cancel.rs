// SPDX-License-Identifier: MIT

use super::{HttpResponse, ReadError, classify_io, deadline, find_header_end, parse_length};
use std::collections::BTreeMap;
use std::net::TcpStream;
use std::time::{Duration, Instant};

const CANCELLATION_POLL: Duration = Duration::from_millis(25);

pub(super) fn read_response_with_limit<F>(
    stream: &mut TcpStream,
    expires: Instant,
    max_response_bytes: usize,
    should_cancel: F,
) -> Result<HttpResponse, ReadError>
where
    F: Fn() -> bool,
{
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 2048];
    let header_end = loop {
        if let Some(end) = find_header_end(&bytes) {
            if end + 4 > super::MAX_HEADER_BYTES {
                return Err(ReadError::Oversized);
            }
            break end;
        }
        if bytes.len() >= super::MAX_HEADER_BYTES {
            return Err(ReadError::Oversized);
        }
        let read = read_with_cancellation(stream, &mut buffer, expires, &should_cancel)?;
        if read == 0 {
            return Err(ReadError::Malformed);
        }
        bytes.extend_from_slice(&buffer[..read]);
    };
    let header = std::str::from_utf8(&bytes[..header_end]).map_err(|_| ReadError::Malformed)?;
    let mut lines = header.split("\r\n");
    let status_line = lines.next().ok_or(ReadError::Malformed)?;
    let mut parts = status_line.split_ascii_whitespace();
    if parts.next() != Some("HTTP/1.1") {
        return Err(ReadError::Malformed);
    }
    let status = parts
        .next()
        .ok_or(ReadError::Malformed)?
        .parse::<u16>()
        .map_err(|_| ReadError::Malformed)?;
    if !(100..=599).contains(&status) {
        return Err(ReadError::Malformed);
    }
    let mut content_length = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(ReadError::Malformed);
        };
        if !super::valid_header(name, value) || name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(ReadError::Malformed);
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(ReadError::Malformed);
            }
            content_length = Some(parse_length(value.trim()).ok_or(ReadError::Malformed)?);
        }
    }
    let content_length = content_length.ok_or(ReadError::Malformed)?;
    if content_length > max_response_bytes {
        return Err(ReadError::Oversized);
    }
    let body_start = header_end + 4;
    let available = bytes.len().saturating_sub(body_start);
    if available > content_length {
        return Err(ReadError::Malformed);
    }
    let mut body = bytes[body_start..].to_vec();
    while body.len() < content_length {
        let remaining = content_length - body.len();
        let read_capacity = remaining.min(buffer.len());
        let read = read_with_cancellation(
            stream,
            &mut buffer[..read_capacity],
            expires,
            &should_cancel,
        )?;
        if read == 0 {
            return Err(ReadError::Malformed);
        }
        body.extend_from_slice(&buffer[..read]);
    }
    Ok(HttpResponse { status, body })
}

fn read_with_cancellation<F>(
    stream: &mut TcpStream,
    bytes: &mut [u8],
    expires: Instant,
    should_cancel: &F,
) -> Result<usize, ReadError>
where
    F: Fn() -> bool,
{
    loop {
        if should_cancel() {
            return Err(ReadError::Cancelled);
        }
        let poll_expires = Instant::now() + CANCELLATION_POLL;
        let read_expires = poll_expires.min(expires);
        let Some(timeout) = read_expires.checked_duration_since(Instant::now()) else {
            return Err(ReadError::Timeout);
        };
        return match deadline::read_with_timeout(stream, bytes, timeout) {
            Ok(_read) if should_cancel() => Err(ReadError::Cancelled),
            Ok(_read) if Instant::now() >= expires => Err(ReadError::Timeout),
            Ok(read) => Ok(read),
            Err(error)
                if error.kind() == std::io::ErrorKind::TimedOut && read_expires < expires =>
            {
                continue;
            }
            Err(error) => Err(classify_io(error)),
        };
    }
}

pub(super) fn write_request<F>(
    stream: &mut TcpStream,
    method: &str,
    path: &str,
    headers: &BTreeMap<String, String>,
    body: &[u8],
    expires: Instant,
    should_cancel: F,
) -> Result<(), ReadError>
where
    F: Fn() -> bool,
{
    if should_cancel() {
        return Err(ReadError::Cancelled);
    }
    let mut request = format!("{method} {path} HTTP/1.1\r\n");
    for (name, value) in headers {
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str("Connection: close\r\n\r\n");
    deadline::write_with_cancellation(stream, request.as_bytes(), expires, &should_cancel)
        .map_err(classify_cancelable_io)?;
    deadline::write_with_cancellation(stream, body, expires, should_cancel)
        .map_err(classify_cancelable_io)
}

fn classify_cancelable_io(error: std::io::Error) -> ReadError {
    if error.kind() == std::io::ErrorKind::Interrupted {
        ReadError::Cancelled
    } else {
        classify_io(error)
    }
}
