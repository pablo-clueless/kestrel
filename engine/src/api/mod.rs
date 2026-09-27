mod guard;
mod hosts;
mod runs;
mod workspace;

use std::sync::Arc;

use axum::{
    Json, Router,
    http::{HeaderName, HeaderValue, Method, header},
    middleware,
    routing::{get, post, put},
};
use serde_json::{Value, json};
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::{config::Config, engine::registry::RunRegistry, model::store::WorkspaceStore};

pub use guard::TOKEN_HEADER;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub runs: Arc<RunRegistry>,
    pub store: Arc<WorkspaceStore>,
    pub hosts: Arc<hosts::ConfirmedHosts>,
}

impl AppState {
    pub fn new(config: Config, store: WorkspaceStore) -> Self {
        Self { config: Arc::new(config), runs: Arc::default(), store: Arc::new(store), hosts: Arc::default() }
    }
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/workspace", get(workspace::get).put(workspace::put))
        .route("/secrets", put(workspace::set_secret))
        .route("/render", post(workspace::render))
        .route("/send", post(workspace::send))
        .route("/hosts", get(hosts::list))
        .route("/hosts/confirm", post(hosts::confirm))
        .route("/runs", get(runs::list).post(runs::start))
        .route("/runs/{id}", axum::routing::delete(runs::stop))
        .route("/runs/{id}/events", get(runs::events))
        .route("/runs/{id}/report", get(runs::report))
        .layer(middleware::from_fn_with_state(state.clone(), guard::guard))
        .with_state(state.clone());

    // CORS is outermost so preflights are answered without a token (browsers never send one).
    Router::new().nest("/api", api).layer(cors(&state.config))
}

fn cors(config: &Config) -> CorsLayer {
    let origins: Vec<HeaderValue> =
        config.allowed_origins.iter().filter_map(|o| HeaderValue::from_str(o).ok()).collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([
            header::CONTENT_TYPE,
            HeaderName::from_static(TOKEN_HEADER),
            HeaderName::from_static("last-event-id"),
        ])
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }))
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;
    use crate::{
        engine::types::{RunEvent, RunReport, RunStatus, StartRunResponse},
        model::{Secrets, Workspace},
    };

    const TOKEN: &str = "test-token";
    const HOST: &str = "127.0.0.1:7070";

    fn app() -> Router {
        app_with(Workspace::default(), Secrets::default())
    }

    fn app_with(workspace: Workspace, secrets: Secrets) -> Router {
        let config = Config::new(7070, TOKEN.into(), false, vec!["http://localhost:3000".into()]);
        router(AppState::new(config, WorkspaceStore::in_memory(workspace, secrets)))
    }

    fn get(uri: &str) -> axum::http::request::Builder {
        Request::get(uri).header(header::HOST, HOST).header(TOKEN_HEADER, TOKEN)
    }

    async fn status(app: &Router, req: Request<Body>) -> StatusCode {
        app.clone().oneshot(req).await.unwrap().status()
    }

    async fn json_body<T: serde::de::DeserializeOwned>(res: axum::response::Response) -> T {
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap()
    }

    #[tokio::test]
    async fn accepts_a_valid_request() {
        assert_eq!(status(&app(), get("/api/health").body(Body::empty()).unwrap()).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn rejects_missing_token() {
        let req = Request::get("/api/health").header(header::HOST, HOST).body(Body::empty()).unwrap();
        assert_eq!(status(&app(), req).await, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn rejects_wrong_token() {
        let req = Request::get("/api/health")
            .header(header::HOST, HOST)
            .header(TOKEN_HEADER, "nope")
            .body(Body::empty())
            .unwrap();
        assert_eq!(status(&app(), req).await, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn rejects_foreign_host_even_with_token() {
        let req = Request::get("/api/health")
            .header(header::HOST, "evil.example:7070")
            .header(TOKEN_HEADER, TOKEN)
            .body(Body::empty())
            .unwrap();
        assert_eq!(status(&app(), req).await, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn rejects_foreign_origin_and_accepts_allowlisted_one() {
        let app = app();
        let foreign = get("/api/health").header(header::ORIGIN, "https://evil.example").body(Body::empty());
        assert_eq!(status(&app, foreign.unwrap()).await, StatusCode::FORBIDDEN);
        let ui = get("/api/health").header(header::ORIGIN, "http://localhost:3000").body(Body::empty());
        assert_eq!(status(&app, ui.unwrap()).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn rejects_non_json_body() {
        let req = Request::post("/api/runs")
            .header(header::HOST, HOST)
            .header(TOKEN_HEADER, TOKEN)
            .header(header::CONTENT_TYPE, "text/plain")
            .body(Body::from(r#"{"kind":"fake","durationMs":1000}"#))
            .unwrap();
        assert_eq!(status(&app(), req).await, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn query_token_only_works_on_the_events_endpoint() {
        let req = Request::get(format!("/api/health?token={TOKEN}"))
            .header(header::HOST, HOST)
            .body(Body::empty())
            .unwrap();
        assert_eq!(status(&app(), req).await, StatusCode::FORBIDDEN);

        // Passes the guard, then 404s because the run doesn't exist.
        let id = uuid::Uuid::new_v4();
        let req = Request::get(format!("/api/runs/{id}/events?token={TOKEN}"))
            .header(header::HOST, HOST)
            .body(Body::empty())
            .unwrap();
        assert_eq!(status(&app(), req).await, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn preflight_from_the_ui_origin_succeeds_without_a_token() {
        let req = Request::options("/api/runs")
            .header(header::HOST, HOST)
            .header(header::ORIGIN, "http://localhost:3000")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "content-type,x-kestrel-token")
            .body(Body::empty())
            .unwrap();
        let res = app().oneshot(req).await.unwrap();
        assert!(res.status().is_success());
        assert_eq!(res.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "http://localhost:3000");
    }

    #[tokio::test]
    async fn rejects_duration_above_cap() {
        let req = Request::post("/api/runs")
            .header(header::HOST, HOST)
            .header(TOKEN_HEADER, TOKEN)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"kind":"fake","durationMs":600000}"#))
            .unwrap();
        assert_eq!(status(&app(), req).await, StatusCode::BAD_REQUEST);
    }

    /// Serves `/echo-headers` on an ephemeral loopback port; returns its base URL.
    async fn echo_headers_server() -> String {
        use axum::http::HeaderMap;
        let app = Router::new().route(
            "/echo-headers",
            axum::routing::get(|headers: HeaderMap| async move {
                let map: std::collections::BTreeMap<String, String> = headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_owned()))
                    .collect();
                Json(map)
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn echo_workspace(base: &str) -> (Workspace, Secrets, uuid::Uuid) {
        use crate::model::{Auth, Body, Collection, Endpoint, Environment, HttpMethod};
        let id = uuid::Uuid::new_v4();
        let workspace = Workspace {
            collections: vec![Collection {
                id: uuid::Uuid::new_v4(),
                name: "echo".into(),
                vars: Default::default(),
                endpoints: vec![Endpoint {
                id,
                name: "echo headers".into(),
                group: None,
                method: HttpMethod::Get,
                url: "{{base}}/echo-headers".into(),
                headers: vec![],
                query: vec![],
                body: Body::None,
                auth: Auth::Bearer { token: "{{token}}".into() },
            }],
            }],
            environments: vec![Environment {
                name: "local".into(),
                vars: [("base".to_string(), base.to_string())].into(),
            }],
            active_environment: Some("local".into()),
            active_collection: None,
        };
        let secrets: Secrets = [("local".to_string(), [("token".to_string(), "hunter2-secret".to_string())].into())].into();
        (workspace, secrets, id)
    }

    fn post_json(uri: &str, body: String) -> Request<Body> {
        Request::post(uri)
            .header(header::HOST, HOST)
            .header(TOKEN_HEADER, TOKEN)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap()
    }

    /// M1 "done when": percentiles + histogram, and the echoed Authorization header is redacted.
    #[tokio::test]
    async fn latency_run_measures_and_redacts_echoed_secrets() {
        let base = echo_headers_server().await;
        let (workspace, secrets, id) = echo_workspace(&base);
        let app = app_with(workspace, secrets);

        let config = format!(
            r#"{{"kind":"latency","endpointId":"{id}","warmup":2,"samples":20,"keepAlive":true,"timeoutMs":5000}}"#
        );
        let res = app.clone().oneshot(post_json("/api/runs", config)).await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let StartRunResponse { run_id } = json_body(res).await;

        // Drain the stream; it ends after `Finished`.
        let events = app
            .clone()
            .oneshot(get(&format!("/api/runs/{run_id}/events")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = String::from_utf8(events.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
        assert!(body.contains(r#""type":"finished""#));
        assert!(!body.contains("hunter2"), "secrets must never reach the event stream");

        let res = app
            .oneshot(get(&format!("/api/runs/{run_id}/report")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let text = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
        assert!(!text.contains("hunter2"), "report leaked the secret: {text}");
        let report: RunReport = serde_json::from_str(&text).unwrap();

        assert_eq!(report.status, RunStatus::Completed);
        assert_eq!(report.total_requests, 20, "warm-up requests are excluded");
        assert_eq!(report.total_errors, 0);
        let latency = report.latency.expect("latency summary");
        assert!(latency.p50_ms > 0.0 && latency.p99_ms >= latency.p50_ms);
        assert_eq!(report.histogram.iter().map(|b| b.count).sum::<u64>(), 20);
        assert!(report.cold_ms.is_some());
        assert!(report.target.unwrap().loopback);

        let sample = &report.samples[0];
        assert_eq!(sample.status, Some(200));
        assert!(sample.body.contains(crate::redact::REDACTED), "{}", sample.body);
        let auth = sample.request.headers.iter().find(|(k, _)| k == "Authorization").unwrap();
        assert_eq!(auth.1, crate::redact::REDACTED);
    }

    #[tokio::test]
    async fn send_redacts_and_render_masks() {
        let base = echo_headers_server().await;
        let (workspace, secrets, _) = echo_workspace(&base);
        let endpoint = serde_json::to_value(&workspace.collections[0].endpoints[0]).unwrap();
        let app = app_with(workspace, secrets);
        let body = serde_json::json!({ "endpoint": endpoint }).to_string();

        let res = app.clone().oneshot(post_json("/api/send", body.clone())).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let text = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
        assert!(text.contains(r#""status":200"#), "{text}");
        assert!(!text.contains("hunter2"), "{text}");

        let res = app.oneshot(post_json("/api/render", body)).await.unwrap();
        let text = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
        assert!(text.contains(crate::template::MASK), "{text}");
        assert!(!text.contains("hunter2"), "{text}");
    }

    #[tokio::test]
    async fn latency_run_with_undefined_variable_is_a_400() {
        let (mut workspace, secrets, id) = echo_workspace("http://127.0.0.1:1");
        workspace.collections[0].endpoints[0].url = "{{nope}}/x".into();
        let app = app_with(workspace, secrets);
        let config = format!(
            r#"{{"kind":"latency","endpointId":"{id}","warmup":0,"samples":1,"keepAlive":true,"timeoutMs":1000}}"#
        );
        let res = app.oneshot(post_json("/api/runs", config)).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body: serde_json::Value = json_body(res).await;
        assert_eq!(body["error"], "undefined variable: nope (define it in environment `local` or the collection)");
    }

    #[tokio::test]
    async fn load_against_unconfirmed_host_is_refused_until_confirmed() {
        // TEST-NET-1: never routed, never resolved, so nothing is sent before confirmation.
        let (workspace, secrets, id) = echo_workspace("http://192.0.2.1:9");
        let app = app_with(workspace, secrets);
        let config = format!(
            r#"{{"kind":"load","endpointId":"{id}","mode":{{"type":"closed","concurrency":1}},"durationMs":100,"timeoutMs":100,"keepAlive":true}}"#
        );
        let res = app.clone().oneshot(post_json("/api/runs", config.clone())).await.unwrap();
        assert_eq!(res.status(), StatusCode::CONFLICT);
        let body: serde_json::Value = json_body(res).await;
        assert_eq!(body["code"], "hostNotConfirmed");
        assert_eq!(body["host"], "192.0.2.1");

        let res = app.clone().oneshot(post_json("/api/hosts/confirm", r#"{"host":"192.0.2.1"}"#.into())).await.unwrap();
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
        let res = app.clone().oneshot(post_json("/api/runs", config)).await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let StartRunResponse { run_id } = json_body(res).await;
        let stop = Request::delete(format!("/api/runs/{run_id}")).header(header::HOST, HOST).header(TOKEN_HEADER, TOKEN).body(Body::empty()).unwrap();
        assert_eq!(status(&app, stop).await, StatusCode::ACCEPTED);
    }

    /// Start a short fake run, read the whole SSE stream, then fetch the report.
    #[tokio::test]
    async fn fake_run_streams_to_finished_and_produces_a_report() {
        let app = app();
        let req = Request::post("/api/runs")
            .header(header::HOST, HOST)
            .header(TOKEN_HEADER, TOKEN)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"kind":"fake","durationMs":600}"#))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let StartRunResponse { run_id } = json_body(res).await;

        let res = app
            .clone()
            .oneshot(get(&format!("/api/runs/{run_id}/events")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();

        let mut ids = Vec::new();
        let mut events = Vec::new();
        for line in body.lines() {
            if let Some(id) = line.strip_prefix("id: ") {
                ids.push(id.parse::<u64>().unwrap());
            } else if let Some(data) = line.strip_prefix("data: ") {
                events.push(serde_json::from_str::<RunEvent>(data).unwrap());
            }
        }
        assert!(matches!(events.first(), Some(RunEvent::Started(_))));
        assert!(matches!(events.last(), Some(RunEvent::Finished(f)) if f.status == RunStatus::Completed));
        assert!(events.iter().filter(|e| matches!(e, RunEvent::Bucket(_))).count() >= 2);
        assert!(ids.windows(2).all(|w| w[1] == w[0] + 1), "ids must be contiguous: {ids:?}");

        // Resuming from the second-to-last id replays only the tail.
        let resume = get(&format!("/api/runs/{run_id}/events"))
            .header("last-event-id", (ids[ids.len() - 2]).to_string())
            .body(Body::empty())
            .unwrap();
        let body = app.clone().oneshot(resume).await.unwrap().into_body().collect().await.unwrap();
        let tail = String::from_utf8(body.to_bytes().to_vec()).unwrap();
        assert_eq!(tail.lines().filter(|l| l.starts_with("data: ")).count(), 1);

        let res = app
            .oneshot(get(&format!("/api/runs/{run_id}/report")).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let report: RunReport = json_body(res).await;
        assert_eq!(report.status, RunStatus::Completed);
        assert!(report.total_requests > 0);
    }
}
