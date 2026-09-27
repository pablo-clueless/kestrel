//! Reference target with known behaviour, so Kestrel can be checked rather than trusted.
//! HANDOFF → Test target. Run with `cargo run -p engine --example target` (port 8080, or
//! `TARGET_PORT`).

use std::{
    collections::{BTreeMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
    hint::black_box,
    net::SocketAddr,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use rand::RngExt;
use serde::Deserialize;

const LIMITED_RPS: u64 = 50;

#[derive(Default)]
struct AppState {
    stall_epoch: OnceLock<Instant>,
    cached: Mutex<HashSet<u64>>,
    /// (window start, requests in window) for `/limited`.
    limiter: Mutex<(Option<Instant>, u64)>,
}

type Shared = State<Arc<AppState>>;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port: u16 = std::env::var("TARGET_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8080);
    let app = Router::new()
        .route("/fast", get(|| async { "ok" }))
        .route("/sleep", get(sleep))
        .route("/stall", get(stall))
        .route("/echo", post(echo))
        .route("/linear", post(linear))
        .route("/quadratic", post(quadratic))
        .route("/sort", post(sort))
        .route("/cached", post(cached))
        .route("/flaky", get(flaky))
        .route("/limited", get(limited))
        .route("/redirect", get(redirect))
        .route("/echo-headers", get(echo_headers))
        .with_state(Arc::new(AppState::default()));

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("target listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

#[derive(Deserialize)]
struct SleepQuery {
    #[serde(default)]
    ms: u64,
}

/// Accurate to well under a millisecond. Plain timer sleeps round up to the OS timer tick (~15.6 ms
/// on Windows), which would make this endpoint useless for checking Kestrel's accuracy.
async fn sleep(Query(q): Query<SleepQuery>) -> String {
    const TIMER_SLACK: Duration = Duration::from_millis(20);
    let deadline = Instant::now() + Duration::from_millis(q.ms.min(60_000));
    if let Some(coarse) = deadline.checked_duration_since(Instant::now() + TIMER_SLACK) {
        tokio::time::sleep(coarse).await;
    }
    // Spin out the remainder off the async workers.
    let _ = tokio::task::spawn_blocking(move || {
        while Instant::now() < deadline {
            std::hint::spin_loop();
        }
    })
    .await;
    format!("slept {} ms", q.ms)
}

/// A server-wide pause, like a GC: for 500 ms out of every 2 s (counted from the first request),
/// every request that arrives waits until the pause ends.
///
/// This is what coordinated omission hides. A closed-model user sends one request into the pause
/// and waits, so the pause shows up once; an open-model test keeps sending during it, so a quarter
/// of all requests are slow — which is what real arrival traffic would see.
async fn stall(State(s): Shared) -> &'static str {
    const PERIOD: Duration = Duration::from_secs(2);
    const PAUSE: Duration = Duration::from_millis(500);
    let epoch = *s.stall_epoch.get_or_init(Instant::now);
    let phase = Duration::from_nanos((epoch.elapsed().as_nanos() % PERIOD.as_nanos()) as u64);
    if phase < PAUSE {
        tokio::time::sleep(PAUSE - phase).await;
    }
    "ok"
}

async fn echo(headers: HeaderMap, body: Bytes) -> Response {
    let content_type = headers.get(header::CONTENT_TYPE).cloned();
    let mut res = body.into_response();
    if let Some(ct) = content_type {
        res.headers_mut().insert(header::CONTENT_TYPE, ct);
    }
    res
}

async fn linear(Json(input): Json<Vec<i64>>) -> Json<i64> {
    Json(input.iter().fold(0i64, |acc, x| black_box(acc.wrapping_add(*x))))
}

fn count_pairs(input: &[i64]) -> u64 {
    let mut pairs = 0u64;
    for a in input {
        for b in input {
            if black_box(a < b) {
                pairs += 1;
            }
        }
    }
    pairs
}

async fn quadratic(Json(input): Json<Vec<i64>>) -> Json<u64> {
    Json(count_pairs(&input))
}

async fn sort(Json(mut input): Json<Vec<i64>>) -> Json<Vec<i64>> {
    input.sort_unstable();
    Json(input)
}

/// O(n²) the first time a body is seen, instant afterwards. Big-O must use fresh values.
async fn cached(State(s): Shared, Json(input): Json<Vec<i64>>) -> Json<bool> {
    let mut hasher = DefaultHasher::new();
    input.hash(&mut hasher);
    let key = hasher.finish();
    if s.cached.lock().unwrap().contains(&key) {
        return Json(true);
    }
    black_box(count_pairs(&input));
    s.cached.lock().unwrap().insert(key);
    Json(false)
}

async fn flaky() -> Response {
    if rand::rng().random_bool(0.1) {
        (StatusCode::INTERNAL_SERVER_ERROR, "flaky failure").into_response()
    } else {
        "ok".into_response()
    }
}

/// 429 once more than 50 requests arrive within one second.
async fn limited(State(s): Shared) -> Response {
    let mut limiter = s.limiter.lock().unwrap();
    let now = Instant::now();
    match limiter.0 {
        Some(start) if now.duration_since(start) < Duration::from_secs(1) => limiter.1 += 1,
        _ => *limiter = (Some(now), 1),
    }
    if limiter.1 > LIMITED_RPS {
        (StatusCode::TOO_MANY_REQUESTS, [(header::RETRY_AFTER, "1")], "slow down").into_response()
    } else {
        "ok".into_response()
    }
}

#[derive(Deserialize)]
struct RedirectQuery {
    to: String,
}

async fn redirect(Query(q): Query<RedirectQuery>) -> Redirect {
    Redirect::to(&q.to)
}

async fn echo_headers(headers: HeaderMap) -> Json<BTreeMap<String, String>> {
    Json(
        headers
            .iter()
            .map(|(k, v)| (k.to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned()))
            .collect(),
    )
}
