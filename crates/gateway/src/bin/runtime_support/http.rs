// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::net::TcpStream;
use std::time::Instant;

#[path = "http_deadline.rs"]
mod deadline;

pub(crate) const MAX_HEADER_BYTES: usize = 8 * 1024;
pub(crate) const MAX_BODY_BYTES: usize = 16 * 1024;
pub(crate) const MAX_RESPONSE_BYTES: usize = 128 * 1024;

pub(crate) struct HttpRequest {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Vec<u8>,
}

pub(crate) struct HttpResponse {
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
}

impl HttpRequest {
    pub(crate) fn content_type_is_json(&self) -> bool {
        self.headers.get("content-type").map(String::as_str) == Some("application/json")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadError {
    Timeout,
    Malformed,
    Oversized,
    Unavailable,
}

pub(crate) fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, u16> {
    read_request_until(stream, Instant::now() + deadline::MESSAGE_TIMEOUT)
}

fn read_request_until(stream: &mut TcpStream, expires: Instant) -> Result<HttpRequest, u16> {
    let mut bytes = Vec::with_capacity(MAX_HEADER_BYTES);
    let mut buffer = [0_u8; 2048];
    let header_end = loop {
        if let Some(end) = find_header_end(&bytes) {
            if end + 4 > MAX_HEADER_BYTES {
                return Err(413);
            }
            break end;
        }
        if bytes.len() >= MAX_HEADER_BYTES {
            return Err(413);
        }
        let read = deadline::read(stream, &mut buffer, expires).map_err(request_io_status)?;
        if read == 0 {
            return Err(400);
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() > MAX_HEADER_BYTES + MAX_BODY_BYTES {
            return Err(413);
        }
    };
    let header = std::str::from_utf8(&bytes[..header_end]).map_err(|_| 400_u16)?;
    let mut lines = header.split("\r\n");
    let request_line = lines.next().ok_or(400_u16)?;
    let mut parts = request_line.split_ascii_whitespace();
    let method = parts.next().ok_or(400_u16)?.to_owned();
    let path = parts.next().ok_or(400_u16)?.to_owned();
    if parts.next() != Some("HTTP/1.1") || parts.next().is_some() {
        return Err(400);
    }
    let mut headers = BTreeMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(400);
        };
        if !valid_header(name, value) {
            return Err(400);
        }
        let name = name.to_ascii_lowercase();
        let value = value.trim();
        if name.is_empty()
            || value.len() > MAX_HEADER_BYTES
            || headers.insert(name, value.to_owned()).is_some()
        {
            return Err(400);
        }
    }
    if headers.contains_key("transfer-encoding") {
        return Err(400);
    }
    let content_length = match headers.get("content-length") {
        Some(value) => parse_length(value).ok_or(400_u16)?,
        None => 0,
    };
    if content_length > MAX_BODY_BYTES {
        return Err(413);
    }
    let body_start = header_end + 4;
    let available = bytes.len().saturating_sub(body_start);
    if available > content_length {
        return Err(400);
    }
    let mut body = bytes[body_start..].to_vec();
    while body.len() < content_length {
        let remaining = content_length - body.len();
        let read_capacity = remaining.min(buffer.len());
        let read = deadline::read(stream, &mut buffer[..read_capacity], expires)
            .map_err(request_io_status)?;
        if read == 0 {
            return Err(400);
        }
        body.extend_from_slice(&buffer[..read]);
    }
    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

pub(crate) fn read_response(
    stream: &mut TcpStream,
    expires: Instant,
) -> Result<HttpResponse, ReadError> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 2048];
    let header_end = loop {
        if let Some(end) = find_header_end(&bytes) {
            if end + 4 > MAX_HEADER_BYTES {
                return Err(ReadError::Oversized);
            }
            break end;
        }
        if bytes.len() >= MAX_HEADER_BYTES {
            return Err(ReadError::Oversized);
        }
        let read = deadline::read(stream, &mut buffer, expires).map_err(classify_io)?;
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
        if !valid_header(name, value) || name.eq_ignore_ascii_case("transfer-encoding") {
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
    if content_length > MAX_RESPONSE_BYTES {
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
        let read =
            deadline::read(stream, &mut buffer[..read_capacity], expires).map_err(classify_io)?;
        if read == 0 {
            return Err(ReadError::Malformed);
        }
        body.extend_from_slice(&buffer[..read]);
    }
    Ok(HttpResponse { status, body })
}

pub(crate) fn write_request(
    stream: &mut TcpStream,
    method: &str,
    path: &str,
    headers: &BTreeMap<String, String>,
    body: &[u8],
    expires: Instant,
) -> std::io::Result<()> {
    let mut request = format!("{method} {path} HTTP/1.1\r\n");
    for (name, value) in headers {
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str("Connection: close\r\n\r\n");
    deadline::write(stream, request.as_bytes(), expires)?;
    deadline::write(stream, body, expires)
}

pub(crate) fn write_response(
    stream: &mut TcpStream,
    status: u16,
    body: &[u8],
) -> std::io::Result<()> {
    let expires = Instant::now() + deadline::MESSAGE_TIMEOUT;
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        408 => "Request Timeout",
        409 => "Conflict",
        413 => "Payload Too Large",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    deadline::write(stream, header.as_bytes(), expires)?;
    deadline::write(stream, body, expires)
}

fn valid_header(name: &str, value: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
        && value
            .bytes()
            .all(|byte| byte == b'\t' || (32..=126).contains(&byte))
}

fn parse_length(value: &str) -> Option<usize> {
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}

fn request_io_status(error: std::io::Error) -> u16 {
    if classify_io(error) == ReadError::Timeout {
        408
    } else {
        400
    }
}

fn classify_io(error: std::io::Error) -> ReadError {
    if matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) {
        ReadError::Timeout
    } else {
        ReadError::Unavailable
    }
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;
