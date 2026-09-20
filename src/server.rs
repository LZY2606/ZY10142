use crate::service::App;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

pub fn serve(app: App, listen: &str) -> Result<(), String> {
    let listener =
        TcpListener::bind(listen).map_err(|error| format!("cannot bind {listen}: {error}"))?;
    let app = std::cell::RefCell::new(app);
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(error) = handle_connection(&app, stream) {
                    eprintln!("request failed: {error}");
                }
            }
            Err(error) => eprintln!("connection failed: {error}"),
        }
    }
    Ok(())
}

fn handle_connection(app: &std::cell::RefCell<App>, mut stream: TcpStream) -> Result<(), String> {
    let mut header_buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end;
    loop {
        let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            return Ok(());
        }
        header_buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = find_header_end(&header_buffer) {
            header_end = position;
            break;
        }
        if header_buffer.len() > 1_048_576 {
            return Err("headers too large".into());
        }
    }
    let request_text = String::from_utf8_lossy(&header_buffer);
    let mut lines = request_text.lines();
    let request_line = lines.next().ok_or("empty request")?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("/");
    let path = target.split('?').next().unwrap_or("/");

    if method == "OPTIONS" {
        write_response(&mut stream, 204, "No Content", "text/plain", b"")?;
        return Ok(());
    }
    if method == "GET" && (path == "/" || path == "/index.html") {
        write_response(
            &mut stream,
            200,
            "OK",
            "text/html; charset=utf-8",
            include_str!("index.html").as_bytes(),
        )?;
        return Ok(());
    }
    if method == "GET" && path == "/api/state" {
        let value = serde_json::to_value(app.borrow_mut().state().map_err(to_string)?)
            .map_err(to_string)?;
        return write_json(&mut stream, 200, &value);
    }
    if method != "POST" {
        return write_json(
            &mut stream,
            405,
            &serde_json::json!({"error":"method not allowed"}),
        );
    }

    let mut body = request_text[header_end..].as_bytes().to_vec();
    let content_length = request_text
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .and_then(|value| value.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    if content_length > 10 * 1024 * 1024 {
        return write_json(
            &mut stream,
            413,
            &serde_json::json!({"error":"request body too large"}),
        );
    }
    while body.len() < content_length {
        let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(content_length);
    let body = String::from_utf8(body).map_err(|error| error.to_string())?;
    let result = route(&mut app.borrow_mut(), path, &body);
    match result {
        Ok(value) => write_json(&mut stream, 200, &value),
        Err(message) => write_json(&mut stream, 400, &serde_json::json!({"error": message})),
    }
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
}

fn route(app: &mut App, path: &str, body: &str) -> Result<serde_json::Value, String> {
    let segments: Vec<&str> = path
        .trim_start_matches('/')
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    match segments.as_slice() {
        ["api", "state"] => Ok(serde_json::to_value(app.state()?).map_err(to_string)?),
        ["api", "records", "import"] => Ok(serde_json::to_value(
            app.import_records(serde_json::from_str(body).map_err(to_string)?)?,
        )
        .map_err(to_string)?),
        ["api", "cases"] => {
            let case_id = app.create_case(serde_json::from_str(body).map_err(to_string)?)?;
            Ok(serde_json::json!({"caseId": case_id}))
        }
        ["api", "cases", case_id, "candidates"] => {
            app.assign_candidates(
                case_id.parse::<i64>().map_err(to_string)?,
                serde_json::from_str(body).map_err(to_string)?,
            )?;
            Ok(serde_json::json!({"ok": true}))
        }
        ["api", "anchors"] => {
            app.save_anchor(serde_json::from_str(body).map_err(to_string)?)?;
            Ok(serde_json::json!({"ok": true}))
        }
        ["api", "cases", case_id, "versions"] => {
            let version_id = app.create_version(
                case_id.parse::<i64>().map_err(to_string)?,
                serde_json::from_str(body).map_err(to_string)?,
            )?;
            Ok(serde_json::json!({"versionId": version_id}))
        }
        ["api", "versions", version_id, "reviews"] => {
            let review_id = app.add_review(
                version_id.parse::<i64>().map_err(to_string)?,
                serde_json::from_str(body).map_err(to_string)?,
            )?;
            Ok(serde_json::json!({"reviewId": review_id}))
        }
        ["api", "versions", version_id, "publish"] => {
            app.publish_version(version_id.parse::<i64>().map_err(to_string)?)?;
            Ok(serde_json::json!({"ok": true}))
        }
        _ => Err("unknown endpoint".into()),
    }
}

fn to_string<T: std::fmt::Display>(error: T) -> String {
    error.to_string()
}

fn write_json(
    stream: &mut TcpStream,
    status: u16,
    value: &serde_json::Value,
) -> Result<(), String> {
    let body = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    write_response(
        stream,
        status,
        status_text(status),
        "application/json; charset=utf-8",
        &body,
    )
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        405 => "Method Not Allowed",
        _ => "Internal Server Error",
    }
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    status_text: &str,
    content_type: &str,
    body: &[u8],
) -> Result<(), String> {
    let headers = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET,POST,OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(headers.as_bytes())
        .map_err(|error| error.to_string())?;
    stream.write_all(body).map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())
}
