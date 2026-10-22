//! One HTTPS POST of a JSON body, answered with the status, the `retry-after`
//! header and the body as text. **The only module that names the HTTP crate**
//! (`tests/layering.rs`), so replacing it is a change here.
//!
//! No retries and no pauses: asking again is the caller's decision, and `core`
//! may not sleep. A status the server sent — 401, 429, 529 — is an answer, not
//! an error, so a caller can tell a refused key from a dropped connection.
//! Proxies come from `HTTPS_PROXY`/`ALL_PROXY`/`NO_PROXY`, as every other
//! program on the machine reads them.

use std::time::Duration;

use anyhow::{Context, Result};

/// What the server said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    /// `retry-after`, in whole seconds, when the server sent one as a number.
    pub retry_after: Option<u64>,
    pub body: String,
}

/// How long a connection may take to open, whatever the exchange's timeout: a
/// host that does not answer at all is said within this, not after minutes.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// POST `body` to `url` with `headers`, waiting at most `timeout` for the
/// whole exchange. An `Err` is a request that never got an answer: no
/// connection, a TLS failure, a timeout, a body that would not read.
pub fn post_json(
    url: &str,
    headers: &[(&str, &str)],
    body: &str,
    timeout: Duration,
) -> Result<Reply> {
    let mut config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(CONNECT_TIMEOUT.min(timeout)))
        .timeout_global(Some(timeout));
    // A proxy cannot reach this machine's own loopback, and a `NO_PROXY` that
    // forgot `127.0.0.1` would send a local test server's request out to it.
    if is_loopback(host_of(url)) {
        config = config.proxy(None);
    }
    let config = config.build();
    let agent = ureq::Agent::new_with_config(config);
    let mut request = agent.post(url).header("content-type", "application/json");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let mut response = request
        .send(body)
        .with_context(|| format!("no answer from {}", host_of(url)))?;
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok());
    let body = response
        .body_mut()
        .read_to_string()
        .with_context(|| format!("reading the answer from {}", host_of(url)))?;
    Ok(Reply {
        status,
        retry_after,
        body,
    })
}

/// The host a URL names, for a sentence: never the path or the query.
fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['/', '?']).next().unwrap_or(rest)
}

/// Whether `host` (with or without its port) is this machine's loopback.
fn is_loopback(host: &str) -> bool {
    let name = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(rest),
        None => host.rsplit_once(':').map_or(host, |(name, _)| name),
    };
    name.eq_ignore_ascii_case("localhost")
        || name
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sentence_names_the_host_alone() {
        assert_eq!(
            host_of("https://api.anthropic.com/v1/messages"),
            "api.anthropic.com"
        );
        assert_eq!(host_of("http://127.0.0.1:4100/v1"), "127.0.0.1:4100");
        assert_eq!(host_of("example.org"), "example.org");
    }

    #[test]
    fn only_this_machine_is_loopback() {
        assert!(is_loopback("127.0.0.1:4100"));
        assert!(is_loopback("localhost"));
        assert!(is_loopback("[::1]:80"));
        assert!(!is_loopback("api.anthropic.com"));
        assert!(!is_loopback("10.0.0.1:443"));
    }

    /// A status the server sent is an answer the caller reads, with its
    /// `retry-after`; only a request that got no answer is an `Err`.
    #[test]
    fn a_refusal_is_an_answer_and_a_dead_port_is_an_error() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut seen = Vec::new();
            let mut buf = [0u8; 4096];
            // Read the head and the body the content-length promises.
            loop {
                let n = stream.read(&mut buf).unwrap();
                seen.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&seen).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if seen.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let body = "{\"type\":\"error\"}";
            write!(
                stream,
                "HTTP/1.1 429 Too Many Requests\r\nretry-after: 7\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
            String::from_utf8_lossy(&seen).to_string()
        });
        let reply = post_json(
            &format!("http://127.0.0.1:{port}/v1/messages"),
            &[("x-test", "yes")],
            "{\"hello\":1}",
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(reply.status, 429);
        assert_eq!(reply.retry_after, Some(7));
        assert_eq!(reply.body, "{\"type\":\"error\"}");
        let request = server.join().unwrap().to_ascii_lowercase();
        assert!(request.starts_with("post /v1/messages"));
        assert!(request.contains("x-test: yes"));
        assert!(request.contains("content-type: application/json"));
        assert!(request.ends_with("{\"hello\":1}"));

        // Nothing listens on a port just let go.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let err = post_json(
            &format!("http://127.0.0.1:{port}/v1"),
            &[],
            "{}",
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("no answer from 127.0.0.1"));
    }
}
