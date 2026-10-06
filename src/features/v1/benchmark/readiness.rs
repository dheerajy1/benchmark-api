use std::{
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};

use crate::config::env::Env;

use super::{
    host::format_authority,
    request::ReadinessRequest,
    target_process::{EXPECTED_STATUS, READINESS_HOST, READINESS_PATH},
};

const CONNECT_TIMEOUT: Duration = Duration::from_millis(100);
const IO_TIMEOUT: Duration = Duration::from_millis(100);

#[derive(Debug, PartialEq, Eq)]
pub enum ReadinessCheck {
    // Server answered with the expected status
    Ready,
    // Server answered with any other status
    Rejected,
    // No usable answer yet, keep polling
    NotReady,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadinessSettings {
    pub host: String,
    pub port: u16,
    pub path: String,
    pub expected_status: u16,
    // None means the .env client credentials are sent
    pub headers: Option<Vec<(String, String)>>,
}

impl ReadinessSettings {
    // Settings used when the port is discovered from the target
    pub fn discovered(port: u16) -> Self {
        Self {
            host: READINESS_HOST.to_string(),
            port,
            path: READINESS_PATH.to_string(),
            expected_status: EXPECTED_STATUS,
            headers: None,
        }
    }

    pub fn from_request(request: &ReadinessRequest) -> Self {
        Self {
            host: request.host.clone(),
            port: request.port,
            path: request.path.clone(),
            expected_status: request.expected_status,
            headers: request.headers.as_ref().map(|headers| {
                headers
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect()
            }),
        }
    }

    // Explicit headers replace the .env defaults
    pub fn resolved_headers(&self, env: &Env) -> Vec<(String, String)> {
        match &self.headers {
            Some(headers) => headers.clone(),
            None => vec![
                ("X-Client-Id".to_string(), env.app_health_client_id.clone()),
                (
                    "X-Client-Secret".to_string(),
                    env.app_health_client_secret.clone(),
                ),
            ],
        }
    }
}

// One readiness probe, never blocks longer than the connect and io timeouts
pub fn check(settings: &ReadinessSettings, env: &Env) -> ReadinessCheck {
    let Some(mut stream) = connect(&settings.host, settings.port) else {
        return ReadinessCheck::NotReady;
    };

    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));

    let request = build_request(settings, env);

    if stream.write_all(request.as_bytes()).is_err() {
        return ReadinessCheck::NotReady;
    }

    let mut response = [0u8; 1024];

    let Ok(bytes_read) = stream.read(&mut response) else {
        return ReadinessCheck::NotReady;
    };

    let response = String::from_utf8_lossy(&response[..bytes_read]);

    parse_status(&response, settings.expected_status)
}

fn connect(host: &str, port: u16) -> Option<TcpStream> {
    let addresses = (host, port).to_socket_addrs().ok()?;

    for address in addresses {
        if let Ok(stream) = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT) {
            return Some(stream);
        }
    }

    None
}

fn build_request(settings: &ReadinessSettings, env: &Env) -> String {
    let authority = format_authority(&settings.host, settings.port);

    let mut request = format!("GET {} HTTP/1.1\r\nHost: {authority}\r\n", settings.path);

    for (name, value) in settings.resolved_headers(env) {
        request.push_str(&format!("{name}: {value}\r\n"));
    }

    request.push_str("Connection: close\r\n\r\n");

    request
}

fn parse_status(response: &str, expected_status: u16) -> ReadinessCheck {
    let Some(status_line) = response.lines().next() else {
        return ReadinessCheck::NotReady;
    };

    let mut parts = status_line.split_whitespace();

    let Some(version) = parts.next() else {
        return ReadinessCheck::NotReady;
    };

    let Some(status) = parts.next() else {
        return ReadinessCheck::NotReady;
    };

    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        return ReadinessCheck::NotReady;
    }

    match status.parse::<u16>() {
        Ok(code) if code == expected_status => ReadinessCheck::Ready,
        Ok(_) => ReadinessCheck::Rejected,
        Err(_) => ReadinessCheck::NotReady,
    }
}
