// SPDX-License-Identifier: MIT

use super::{MAX_HEADER_BYTES, ReadError, find_header_end, read_request_until, read_response};
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

#[test]
fn overload_responses_include_bounded_retry_guidance() {
    let header = super::response_header(429, 17);
    assert!(header.contains("Retry-After: 1\r\n"));
    assert!(header.contains("Content-Length: 17\r\n"));
    assert!(!super::response_header(503, 17).contains("Retry-After:"));
}

fn pair() -> std::io::Result<(TcpStream, TcpStream)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let client = TcpStream::connect(listener.local_addr()?)?;
    let (server, _) = listener.accept()?;
    Ok((client, server))
}

#[test]
fn detects_only_crlf_header_termination() {
    assert_eq!(find_header_end(b"GET / HTTP/1.1\r\n\r\nbody"), Some(14));
    assert_eq!(find_header_end(b"GET / HTTP/1.1\n\n"), None);
}

#[test]
fn stalled_request_and_incomplete_body_expire() -> std::io::Result<()> {
    for prefix in ["", "POST / HTTP/1.1\r\nContent-Length: 2\r\n\r\nx"] {
        let (mut client, mut server) = pair()?;
        client.write_all(prefix.as_bytes())?;
        let start = Instant::now();
        let result = read_request_until(&mut server, start + Duration::from_millis(40));
        assert_eq!(result.err(), Some(408));
        assert!(start.elapsed() < Duration::from_secs(1));
    }
    Ok(())
}

#[test]
fn response_body_drip_does_not_reset_deadline() -> std::io::Result<()> {
    let (mut client, mut server) = pair()?;
    server.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n")?;
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        server.set_write_timeout(Some(Duration::from_millis(100)))?;
        for _ in 0..100 {
            if server.write_all(b"x").is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    });
    let start = Instant::now();
    let result = read_response(&mut client, start + Duration::from_millis(60));
    assert_eq!(result.err(), Some(ReadError::Timeout));
    assert!(start.elapsed() < Duration::from_millis(700));
    drop(client);
    assert!(matches!(writer.join(), Ok(Ok(()))));
    Ok(())
}

#[test]
fn request_header_drip_does_not_reset_deadline() -> std::io::Result<()> {
    let (mut client, mut server) = pair()?;
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        client.set_write_timeout(Some(Duration::from_millis(100)))?;
        for byte in b"POST / HTTP/1.1\r\nContent-Length: 0\r\n\r\n" {
            if client.write_all(&[*byte]).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    });
    let start = Instant::now();
    assert_eq!(
        read_request_until(&mut server, start + Duration::from_millis(60)).err(),
        Some(408),
    );
    assert!(start.elapsed() < Duration::from_millis(700));
    drop(server);
    assert!(matches!(writer.join(), Ok(Ok(()))));
    Ok(())
}

#[test]
fn stalled_peer_cannot_block_a_large_write_forever() -> std::io::Result<()> {
    let (mut client, _server) = pair()?;
    let bytes = vec![b'x'; 16 * 1024 * 1024];
    let start = Instant::now();
    let result = super::deadline::write(&mut client, &bytes, start + Duration::from_millis(40));
    assert!(matches!(result, Err(error) if matches!(error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock)));
    assert!(start.elapsed() < Duration::from_secs(1));
    Ok(())
}

#[test]
fn oversized_terminated_headers_are_rejected() -> std::io::Result<()> {
    for response in [false, true] {
        let (mut client, mut server) = pair()?;
        let start = if response {
            "HTTP/1.1 200 OK"
        } else {
            "GET / HTTP/1.1"
        };
        let mut bytes = format!("{start}\r\nContent-Length: 0\r\nX-Padding: ");
        bytes.extend(std::iter::repeat_n('x', MAX_HEADER_BYTES - bytes.len() - 2));
        bytes.push_str("\r\n\r\n");
        client.write_all(bytes.as_bytes())?;
        let deadline = Instant::now() + Duration::from_secs(1);
        if response {
            assert_eq!(
                read_response(&mut server, deadline).err(),
                Some(ReadError::Oversized)
            );
        } else {
            assert_eq!(read_request_until(&mut server, deadline).err(), Some(413));
        }
    }
    Ok(())
}

#[test]
fn ambiguous_and_invalid_framing_is_rejected() -> std::io::Result<()> {
    for headers in [
        "Transfer-Encoding: chunked\r\nContent-Length: 0",
        "Content-Length: +0",
        "Content-Length : 0",
        "Content-Length: 0\r\ncontent-length: 0",
        "Content-Length: 0\r\nBad Header: x",
    ] {
        for response in [false, true] {
            let (mut client, mut server) = pair()?;
            let first = if response {
                "HTTP/1.1 200 OK"
            } else {
                "POST / HTTP/1.1"
            };
            client.write_all(format!("{first}\r\n{headers}\r\n\r\n").as_bytes())?;
            let deadline = Instant::now() + Duration::from_secs(1);
            if response {
                assert_eq!(
                    read_response(&mut server, deadline).err(),
                    Some(ReadError::Malformed)
                );
            } else {
                assert_eq!(read_request_until(&mut server, deadline).err(), Some(400));
            }
        }
    }
    Ok(())
}

#[test]
fn expired_deadline_prevents_a_request_write() -> std::io::Result<()> {
    let (mut client, _server) = pair()?;
    let result = super::write_request(
        &mut client,
        "POST",
        "/",
        &Default::default(),
        b"body",
        Instant::now(),
    );
    assert!(matches!(result, Err(error) if error.kind() == std::io::ErrorKind::TimedOut));
    Ok(())
}

#[test]
fn exact_header_limit_and_complete_bodies_are_accepted() -> std::io::Result<()> {
    let (mut client, mut server) = pair()?;
    let mut bytes = String::from("POST / HTTP/1.1\r\nContent-Length: 2\r\nX-Padding: ");
    bytes.extend(std::iter::repeat_n('x', MAX_HEADER_BYTES - bytes.len() - 4));
    bytes.push_str("\r\n\r\n{}");
    client.write_all(bytes.as_bytes())?;
    let request = read_request_until(&mut server, Instant::now() + Duration::from_secs(1))
        .map_err(|status| std::io::Error::other(format!("rejected valid request: {status}")))?;
    assert_eq!(request.body, b"{}");
    Ok(())
}
