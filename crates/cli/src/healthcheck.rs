//! Dependency-free HTTP probe so minimal container images (no curl/shell)
//! can run `HEALTHCHECK`.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::process::ExitCode;
use std::time::Duration;

use clap::Args;

const TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Args)]
pub struct HealthcheckArgs {
    /// Health endpoint to probe (plain HTTP).
    #[arg(long, env = "HEALTHCHECK_URL")]
    url: Option<String>,
    /// Port used when --url is not given.
    #[arg(long, env = "PORT", default_value_t = 3005)]
    port: u16,
}

pub fn run(args: &HealthcheckArgs) -> ExitCode {
    let url = args
        .url
        .clone()
        .unwrap_or_else(|| format!("http://127.0.0.1:{}/api/health", args.port));
    match probe(&url) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("unhealthy: {message}");
            ExitCode::FAILURE
        }
    }
}

fn probe(url: &str) -> Result<(), String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or("only http:// URLs are supported")?;
    let (authority, path) = rest
        .split_once('/')
        .map_or((rest, "/".to_owned()), |(a, p)| (a, format!("/{p}")));
    let address = if authority.contains(':') {
        authority.to_owned()
    } else {
        format!("{authority}:80")
    };
    let socket = address
        .to_socket_addrs()
        .map_err(|e| format!("{address}: {e}"))?
        .next()
        .ok_or_else(|| format!("{address}: no address"))?;
    let mut stream =
        TcpStream::connect_timeout(&socket, TIMEOUT).map_err(|e| format!("{address}: {e}"))?;
    stream
        .set_read_timeout(Some(TIMEOUT))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(TIMEOUT))
        .map_err(|e| e.to_string())?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {authority}\r\nUser-Agent: specsentry-healthcheck\r\nConnection: close\r\n\r\n"
    )
    .map_err(|e| e.to_string())?;
    let mut head = [0_u8; 32];
    let read = stream.read(&mut head).map_err(|e| e.to_string())?;
    let status_line = String::from_utf8_lossy(&head[..read]);
    match status_line.split_whitespace().nth(1) {
        Some("200") => Ok(()),
        Some(code) => Err(format!("HTTP {code}")),
        None => Err("malformed response".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn serve_once(response: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("addr");
        std::thread::spawn(move || {
            if let Ok((mut socket, _)) = listener.accept() {
                let mut buf = [0_u8; 512];
                let _ = socket.read(&mut buf);
                let _ = socket.write_all(response.as_bytes());
            }
        });
        format!("http://{address}/api/health")
    }

    #[test]
    fn accepts_200() {
        let url = serve_once("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        assert_eq!(probe(&url), Ok(()));
    }

    #[test]
    fn rejects_other_statuses() {
        let url = serve_once("HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n");
        assert_eq!(probe(&url), Err("HTTP 503".to_owned()));
    }

    #[test]
    fn rejects_unsupported_urls() {
        assert!(probe("https://example.com").is_err());
    }
}
