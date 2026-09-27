//! HTTP client construction and single-request execution with timing.

use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use bytes::Bytes;
use reqwest::{Client, redirect};
use tokio::time::Instant;
use url::{Host, Url};

use super::types::ErrorClass;
use crate::template::request::RenderedRequest;

const MAX_REDIRECTS: usize = 5;

pub struct ClientOptions {
    pub keep_alive: bool,
    /// Load-type tests never follow redirects; 3xx responses are recorded as-is.
    pub follow_redirects: bool,
    /// Hosts the user confirmed they may test; redirects to them are followed too.
    pub confirmed_hosts: Vec<String>,
}

pub struct Target {
    pub client: Client,
    /// The IP every request in this run goes to.
    pub pinned: IpAddr,
}

/// Resolves the target host once and pins it for the client's lifetime, so the address that was
/// checked is the one that gets traffic.
pub async fn connect(url: &Url, opts: &ClientOptions) -> Result<Target, String> {
    let host = url.host_str().ok_or_else(|| format!("URL `{url}` has no host"))?.to_owned();
    let port = url.port_or_known_default().unwrap_or(80);

    let pinned = match url.host() {
        Some(Host::Ipv4(ip)) => IpAddr::V4(ip),
        Some(Host::Ipv6(ip)) => IpAddr::V6(ip),
        _ => tokio::net::lookup_host((host.as_str(), port))
            .await
            .map_err(|e| format!("couldn't resolve `{host}`: {e}"))?
            .next()
            .ok_or_else(|| format!("`{host}` resolved to no addresses"))?
            .ip(),
    };

    let redirects = if opts.follow_redirects {
        let mut allowed = opts.confirmed_hosts.clone();
        allowed.push(host.clone());
        redirect_policy(allowed)
    } else {
        redirect::Policy::none()
    };
    let builder = Client::builder()
        .resolve(&host, SocketAddr::new(pinned, port))
        .redirect(redirects)
        // Keep-alive off = no idle connections, so every request opens a new one.
        .pool_max_idle_per_host(if opts.keep_alive { usize::MAX } else { 0 });
    let client = builder.build().map_err(|e| format!("couldn't build HTTP client: {e}"))?;
    Ok(Target { client, pinned })
}

/// Follows up to 5 redirects, only to `allowed` hosts (the original one plus confirmed ones) or to
/// loopback. Anything else is returned as the 3xx response (HANDOFF → Safety rails → Targets).
fn redirect_policy(allowed: Vec<String>) -> redirect::Policy {
    redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        let host_ok = attempt.url().host_str().is_some_and(|h| allowed.iter().any(|a| a.eq_ignore_ascii_case(h)));
        if host_ok || is_loopback_url(attempt.url()) { attempt.follow() } else { attempt.stop() }
    })
}

fn is_loopback_url(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

pub struct Outcome {
    pub status: Option<u16>,
    /// Until response headers arrived.
    pub ttfb: Duration,
    /// Until the body was fully read. Equals the timeout for timed-out requests.
    pub total: Duration,
    pub response_headers: Vec<(String, String)>,
    pub body: Bytes,
    pub error: Option<(ErrorClass, String)>,
}

impl Outcome {
    pub fn is_success(&self) -> bool {
        self.error.is_none()
    }

    /// Applies the test's error classification: `ok_statuses` count as success.
    pub fn classify_with(mut self, ok_statuses: &[u16]) -> Self {
        if matches!(self.error, Some((ErrorClass::Http | ErrorClass::RateLimited, _)))
            && self.status.is_some_and(|s| ok_statuses.contains(&s))
        {
            self.error = None;
        }
        self
    }
}

/// Sends one request and reads the whole body. Never panics or returns early: every failure mode is
/// an `Outcome` so it lands in the "all requests" histogram.
pub async fn execute(client: &Client, req: &RenderedRequest, timeout: Duration) -> Outcome {
    let mut builder = client.request(req.method.into(), req.url.clone());
    for (k, v) in &req.headers {
        builder = builder.header(k, v);
    }
    if let Some(body) = &req.body {
        builder = builder.body(body.clone());
    }

    let start = Instant::now();
    let deadline = start + timeout;
    let failed = |class, msg: String, ttfb: Option<Duration>| Outcome {
        status: None,
        ttfb: ttfb.unwrap_or_else(|| start.elapsed()),
        total: start.elapsed(),
        response_headers: vec![],
        body: Bytes::new(),
        error: Some((class, msg)),
    };
    let timed_out = |ttfb: Option<Duration>| Outcome {
        total: timeout,
        ..failed(ErrorClass::Timeout, format!("timed out after {} ms", timeout.as_millis()), ttfb.or(Some(timeout)))
    };

    let response = match tokio::time::timeout_at(deadline, builder.send()).await {
        Err(_) => return timed_out(None),
        Ok(Err(err)) => {
            let (class, msg) = classify(&err);
            return failed(class, msg, None);
        }
        Ok(Ok(response)) => response,
    };
    let ttfb = start.elapsed();
    let status = response.status().as_u16();
    let response_headers = response
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_owned(), String::from_utf8_lossy(v.as_bytes()).into_owned()))
        .collect();

    let body = match tokio::time::timeout_at(deadline, response.bytes()).await {
        Err(_) => return timed_out(Some(ttfb)),
        Ok(Err(err)) => {
            let (class, msg) = classify(&err);
            return Outcome { status: Some(status), ..failed(class, msg, Some(ttfb)) };
        }
        Ok(Ok(body)) => body,
    };

    let error = match status {
        429 => Some((ErrorClass::RateLimited, "429 Too Many Requests".to_owned())),
        400..=599 => Some((ErrorClass::Http, format!("HTTP {status}"))),
        _ => None,
    };
    Outcome { status: Some(status), ttfb, total: start.elapsed(), response_headers, body, error }
}

fn classify(err: &reqwest::Error) -> (ErrorClass, String) {
    let msg = error_chain(err);
    if err.is_builder() {
        return (ErrorClass::InvalidRequest, msg);
    }
    if err.is_timeout() {
        return (ErrorClass::Timeout, msg);
    }
    // Ephemeral port exhaustion is the client's problem, not the target's.
    let mut source: Option<&dyn std::error::Error> = Some(err);
    while let Some(e) = source {
        if let Some(io) = e.downcast_ref::<std::io::Error>()
            && io.kind() == std::io::ErrorKind::AddrNotAvailable
        {
            return (ErrorClass::ClientResource, msg);
        }
        source = e.source();
    }
    (ErrorClass::Transport, msg)
}

/// reqwest's top-level message is often just "error sending request"; the cause is further down.
fn error_chain(err: &dyn std::error::Error) -> String {
    let mut parts = vec![err.to_string()];
    let mut source = err.source();
    while let Some(e) = source {
        let s = e.to_string();
        if !parts.iter().any(|p| p.contains(&s)) {
            parts.push(s);
        }
        source = e.source();
    }
    parts.join(": ")
}
