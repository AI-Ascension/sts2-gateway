// SPDX-License-Identifier: MIT

use super::{HttpResponse, ReadError, classify_io, deadline, find_header_end, parse_length};
use std::net::TcpStream;
use std::time::Instant;

pub(super) fn read_response_with_limit(
    stream: &mut TcpStream,
    expires: Instant,
    max_response_bytes: usize,
) -> Result<HttpResponse, ReadError> {
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
        let read =
            deadline::read(stream, &mut buffer[..read_capacity], expires).map_err(classify_io)?;
        if read == 0 {
            return Err(ReadError::Malformed);
        }
        body.extend_from_slice(&buffer[..read]);
    }
    Ok(HttpResponse { status, body })
}
