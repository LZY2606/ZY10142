use crate::db::Repository;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

pub fn serve(repository: Repository, listen: &str) -> Result<(), String> {
    let listener = TcpListener::bind(listen).map_err(|error| error.to_string())?;
    let repository = Arc::new(repository);
    for stream in listener.incoming() {
        let stream = stream.map_err(|error| error.to_string())?;
        if let Err(error) = handle_connection(stream, repository.clone()) {
            eprintln!("request failed: {error}");
        }
    }
    Ok(())
}

fn handle_connection(mut stream: TcpStream, repository: Arc<Repository>) -> Result<(), String> {
    let mut request = String::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        request.push_str(&String::from_utf8_lossy(&chunk[..read]));
        if request.contains("\r\n\r\n") {
            let headers = request.split("\r\n\r\n").next().unwrap_or_default();
            let expected = headers
                .lines()
                .find_map(|line| {
                    line.strip_prefix("Content-Length: ")
                        .or_else(|| line.strip_prefix("content-length: "))
                })
                .and_then(|value| value.trim().parse::<usize>().ok());
            let body_start = headers.len() + 4;
            if let Some(length) = expected {
                while request.len() < body_start + length {
                    let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
                    if read == 0 {
                        break;
                    }
                    request.push_str(&String::from_utf8_lossy(&chunk[..read]));
                }
            }
            break;
        }
    }
    let first_line = request.lines().next().unwrap_or_default();
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let path = parts.next().unwrap_or("/");

    let (status, content_type, body) = if method == "GET" && (path == "/" || path == "/index.html")
    {
        (
            "200 OK".to_string(),
            "text/html; charset=utf-8",
            include_str!("../web/index.html").to_string(),
        )
    } else if method == "GET" && path == "/api/state" {
        match repository.state_with_order() {
            Ok(state) => json_response("200 OK", &state),
            Err(error) => error_response("500 Internal Server Error", &error),
        }
    } else if method == "POST" && path == "/api/batches" {
        parse_and_create(&request, |input| repository.import_batch(input))
    } else if method == "POST" && path == "/api/disturbances" {
        parse_and_create(&request, |input| repository.create_disturbance(input))
    } else if method == "POST" && path == "/api/candidates" {
        parse_and_create(&request, |input| repository.add_candidate(input))
    } else if method == "POST" && path == "/api/publish-alignment" {
        parse_and_create(&request, |input| repository.publish_alignment(input, None))
    } else {
        error_response("404 Not Found", "not found")
    };

    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|error| error.to_string())
}

fn parse_and_create<T, R, F>(request: &str, handler: F) -> (String, &'static str, String)
where
    T: serde::de::DeserializeOwned,
    R: serde::Serialize,
    F: FnOnce(T) -> Result<R, String>,
{
    match request_body(request)
        .and_then(|body| serde_json::from_str::<T>(body).map_err(|error| error.to_string()))
    {
        Ok(input) => match handler(input) {
            Ok(value) => json_response("200 OK", &value),
            Err(error) => error_response("409 Conflict", &error),
        },
        Err(error) => error_response("400 Bad Request", &error),
    }
}

fn request_body(request: &str) -> Result<&str, String> {
    request
        .split("\r\n\r\n")
        .nth(1)
        .filter(|body| !body.trim().is_empty())
        .ok_or_else(|| "JSON request body is required".to_string())
}

fn json_response<R: serde::Serialize>(
    status: &'static str,
    value: &R,
) -> (String, &'static str, String) {
    match serde_json::to_string_pretty(value) {
        Ok(body) => (status.to_string(), "application/json", body),
        Err(error) => error_response("500 Internal Server Error", &error.to_string()),
    }
}

fn error_response(status: &'static str, message: &str) -> (String, &'static str, String) {
    let body = serde_json::json!({ "error": message }).to_string();
    (status.to_string(), "application/json", body)
}
