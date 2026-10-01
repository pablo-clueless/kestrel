mod guard;
mod hosts;
mod import;
mod runs;
mod scope;
mod ui;
mod workspace;

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::DefaultBodyLimit,
    http::{HeaderName, HeaderValue, Method, header},
    middleware,
    routing::{get, post, put},
};
use serde_json::{Value, json};
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::{config::Config, engine::registry::RunRegistry, model::workspaces::Workspaces};

pub use guard::TOKEN_HEADER;
pub use scope::{Scope, WORKSPACE_HEADER};

/// Largest file accepted for multipart file fields.
const MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub runs: Arc<RunRegistry>,
    pub workspaces: Arc<Workspaces>,
}

impl AppState {
    pub fn new(config: Config, workspaces: Workspaces) -> Self {
        Self { config: Arc::new(config), runs: Arc::default(), workspaces: Arc::new(workspaces) }
    }
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/workspace", get(workspace::get).put(workspace::put))
        .route("/secrets", put(workspace::set_secret))
        .route("/render", post(workspace::render))
        .route("/send", post(workspace::send))
        .route("/files", post(workspace::upload_file).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)))
        .route("/import", post(import::import))
        .route("/hosts", get(hosts::list))
        .route("/hosts/confirm", post(hosts::confirm))
        .route("/runs", get(runs::list).post(runs::start))
        .route("/runs/{id}", axum::routing::delete(runs::stop))
        .route("/runs/{id}/events", get(runs::events))
        .route("/runs/{id}/report", get(runs::report))
        .layer(middleware::from_fn_with_state(state.clone(), guard::guard))
        .with_state(state.clone());

    // CORS is outermost so preflights are answered without a token (browsers never send one).
    Router::new()
        .nest("/api", api)
        // Liveness for container health checks: no token, and it reveals nothing.
        .route("/healthz", get(|| async { "ok" }))
        // Everything else is the built UI.
        .fallback(ui::serve)
        .with_state(state.clone())
        .layer(cors(&state.config))
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
            HeaderName::from_static(WORKSPACE_HEADER),
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
        db::{Db, crypto::SecretsCipher, test::require_db},
        engine::types::{RunEvent, RunReport, RunStatus, RunSummary, StartRunResponse},
        model::{Secrets, Workspace},
    };

    const TOKEN: &str = "test-token";
    const HOST: &str = "127.0.0.1:7070";
    /// The workspace test requests act on; `app_with` seeds it. Each test's database has its own
    /// schema prefix, so tests sharing this id still never share data.
    const WS: &str = "00000000-0000-4000-8000-000000000001";

    fn config() -> Config {
        Config::new(7070, TOKEN.into(), false, vec!["http://localhost:3000".into()])
    }

    /// For tests that are decided before any workspace is opened (the guard, health, the UI): the
    /// database is never connected to, so these run without one.
    fn app() -> Router {
        let db = Db::lazy("postgres://unused@127.0.0.1:1/unused", 1, SecretsCipher::for_tests(), "x_").unwrap();
        router(AppState::new(config(), Workspaces::new(Arc::new(db))))
    }

    /// A router over `db` with workspace `WS` holding `workspace` and `secrets`.
    async fn app_with(db: &Arc<Db>, workspace: Workspace, secrets: Secrets) -> Router {
        app_on(db, config(), workspace, secrets).await
    }

    async fn app_on(db: &Arc<Db>, config: Config, workspace: Workspace, secrets: Secrets) -> Router {
        let workspaces = Workspaces::new(Arc::clone(db));
        let store = workspaces.get(WS.parse().unwrap()).await.unwrap();
        store.save_workspace(workspace).await.unwrap();
        for (env, kv) in secrets {
            for (key, value) in kv {
                store.set_secret(&env, &key, Some(value)).await.unwrap();
            }
        }
        router(AppState::new(config, workspaces))
    }

    async fn empty_app(db: &Arc<Db>) -> Router {
        app_with(db, Workspace::default(), Secrets::default()).await
    }

    fn get(uri: &str) -> axum::http::request::Builder {
        Request::get(uri).header(header::HOST, HOST).header(TOKEN_HEADER, TOKEN).header(WORKSPACE_HEADER, WS)
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
    async fn healthz_needs_no_token() {
        let req = Request::get("/healthz").header(header::HOST, "anything:1").body(Body::empty()).unwrap();
        assert_eq!(status(&app(), req).await, StatusCode::OK);
    }

    /// Works whether or not the UI has been built (it isn't in CI).
    #[tokio::test]
    async fn serves_the_ui_with_the_token_or_explains_how_to_build_it() {
        let res = app().oneshot(Request::get("/").body(Body::empty()).unwrap()).await.unwrap();
        let status = res.status();
        let body = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
        match status {
            StatusCode::OK => assert!(body.contains(r#"<meta name="kestrel-token" content="test-token">"#)),
            StatusCode::NOT_FOUND => assert!(body.contains("pnpm build"), "{body}"),
            other => panic!("unexpected {other}"),
        }
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
            .header(WORKSPACE_HEADER, WS)
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
    async fn accepts_an_allowed_deployment_host_over_https() {
        let db = require_db!();
        let mut config = Config::new(7070, TOKEN.into(), false, vec![]);
        config.allow_host("kestrel.example.com");
        let app = app_on(&db, config, Workspace::default(), Secrets::default()).await;
        let req = |host: &str, origin: &str| {
            Request::put("/api/workspace")
                .header(header::HOST, host)
                .header(header::ORIGIN, origin)
                .header(TOKEN_HEADER, TOKEN)
                .header(WORKSPACE_HEADER, WS)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap()
        };
        assert_eq!(
            status(&app, req("kestrel.example.com", "https://kestrel.example.com")).await,
            StatusCode::NO_CONTENT
        );
        assert_eq!(status(&app, req("other.example.com", "https://other.example.com")).await, StatusCode::FORBIDDEN);
    }

    /// Rejected by the guard, before any workspace is opened.
    #[tokio::test]
    async fn rejects_non_json_body() {
        let body = r#"{"kind":"fake","durationMs":1000}"#;
        let req = Request::post("/api/runs")
            .header(header::HOST, HOST)
            .header(TOKEN_HEADER, TOKEN)
            .header(WORKSPACE_HEADER, WS)
            .header(header::CONTENT_TYPE, "text/plain")
            // As a real client sends it; the guard keys its content-type check off the body's length.
            .header(header::CONTENT_LENGTH, body.len())
            .body(Body::from(body))
            .unwrap();
        assert_eq!(status(&app(), req).await, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn query_token_only_works_on_the_events_endpoint() {
        let req =
            Request::get(format!("/api/health?token={TOKEN}")).header(header::HOST, HOST).body(Body::empty()).unwrap();
        assert_eq!(status(&app(), req).await, StatusCode::FORBIDDEN);

        // Passes the guard, then 404s because the run doesn't exist.
        let db = require_db!();
        let id = uuid::Uuid::new_v4();
        let req = Request::get(format!("/api/runs/{id}/events?token={TOKEN}&workspace={WS}"))
            .header(header::HOST, HOST)
            .body(Body::empty())
            .unwrap();
        assert_eq!(status(&empty_app(&db).await, req).await, StatusCode::NOT_FOUND);
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
        let db = require_db!();
        let req = Request::post("/api/runs")
            .header(header::HOST, HOST)
            .header(TOKEN_HEADER, TOKEN)
            .header(WORKSPACE_HEADER, WS)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"kind":"fake","durationMs":600000}"#))
            .unwrap();
        assert_eq!(status(&empty_app(&db).await, req).await, StatusCode::BAD_REQUEST);
    }

    /// Serves `/echo-headers` on an ephemeral loopback port; returns its base URL.
    async fn echo_headers_server() -> String {
        use axum::http::HeaderMap;
        let app = Router::new().route(
            "/echo-headers",
            axum::routing::get(|headers: HeaderMap| async move {
                let map: std::collections::BTreeMap<String, String> =
                    headers.iter().map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_owned())).collect();
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
                source: None,
                schema_defs: None,
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
                    expect: None,
                    extract: vec![],
                }],
            }],
            environments: vec![Environment {
                name: "local".into(),
                vars: [("base".to_string(), base.to_string())].into(),
            }],
            active_environment: Some("local".into()),
            active_collection: None,
        };
        let secrets: Secrets =
            [("local".to_string(), [("token".to_string(), "hunter2-secret".to_string())].into())].into();
        (workspace, secrets, id)
    }

    fn post_json(uri: &str, body: String) -> Request<Body> {
        Request::post(uri)
            .header(header::HOST, HOST)
            .header(TOKEN_HEADER, TOKEN)
            .header(WORKSPACE_HEADER, WS)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap()
    }

    /// M1 "done when": percentiles + histogram, and the echoed Authorization header is redacted.
    #[tokio::test]
    async fn latency_run_measures_and_redacts_echoed_secrets() {
        let base = echo_headers_server().await;
        let (workspace, secrets, id) = echo_workspace(&base);
        let db = require_db!();
        let app = app_with(&db, workspace, secrets).await;

        let config = format!(
            r#"{{"kind":"latency","endpointId":"{id}","warmup":2,"samples":20,"keepAlive":true,"timeoutMs":5000}}"#
        );
        let res = app.clone().oneshot(post_json("/api/runs", config)).await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let StartRunResponse { run_id } = json_body(res).await;

        // Drain the stream; it ends after `Finished`.
        let events =
            app.clone().oneshot(get(&format!("/api/runs/{run_id}/events")).body(Body::empty()).unwrap()).await.unwrap();
        let body = String::from_utf8(events.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
        assert!(body.contains(r#""type":"finished""#));
        assert!(!body.contains("hunter2"), "secrets must never reach the event stream");

        let res = app.oneshot(get(&format!("/api/runs/{run_id}/report")).body(Body::empty()).unwrap()).await.unwrap();
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
    async fn uploads_files_of_any_type() {
        let db = require_db!();
        let app = empty_app(&db).await;
        let upload = |name: &str| {
            Request::post(format!("/api/files?name={name}"))
                .header(header::HOST, HOST)
                .header(TOKEN_HEADER, TOKEN)
                .header(WORKSPACE_HEADER, WS)
                .header(header::CONTENT_TYPE, "image/png")
                .body(Body::from(vec![0x89, b'P', b'N', b'G']))
                .unwrap()
        };
        let res = app.clone().oneshot(upload("cat.png")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let file: crate::model::FileRef = json_body(res).await;
        assert_eq!((file.name.as_str(), file.content_type.as_str(), file.size), ("cat.png", "image/png", 4));
        assert_eq!(status(&app, upload("%20")).await, StatusCode::BAD_REQUEST, "a name is required");

        let no_token = Request::post("/api/files?name=x").header(header::HOST, HOST).body(Body::from("x")).unwrap();
        assert_eq!(status(&app, no_token).await, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn send_runs_extract_rules() {
        let base = echo_headers_server().await;
        let (workspace, secrets, _) = echo_workspace(&base);
        let mut endpoint = serde_json::to_value(&workspace.collections[0].endpoints[0]).unwrap();
        endpoint["extract"] = serde_json::json!([
            { "source": "body", "path": "host", "target": "secret", "name": "echoedHost" },
            { "source": "header", "path": "Content-Type", "target": "variable", "name": "ct" },
            { "source": "body", "path": "no.such", "target": "variable", "name": "missing" },
            { "source": "status", "target": "variable", "name": "skipped", "enabled": false },
        ]);
        let db = require_db!();
        let app = app_with(&db, workspace, secrets).await;
        let body = serde_json::json!({ "endpoint": endpoint, "environment": "local" }).to_string();

        let res = app.clone().oneshot(post_json("/api/send", body)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let text = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
        let host = base.trim_start_matches("http://");
        assert!(!text.contains(host), "a secret saved from this response is redacted in it too: {text}");
        let res: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(res["status"], 200, "the sample is flattened into the response");

        let saved = res["saved"].as_array().unwrap();
        assert_eq!(saved.len(), 3, "disabled rules don't run: {saved:?}");
        assert_eq!(saved[0]["target"], "secret");
        assert!(saved[0]["value"].is_null() && saved[0]["error"].is_null(), "secret values aren't returned");
        assert_eq!(saved[1]["value"], "application/json");
        assert!(saved[2]["error"].as_str().unwrap().contains("not found"));

        let res = app.oneshot(get("/api/workspace").body(Body::empty()).unwrap()).await.unwrap();
        let ws: serde_json::Value = json_body(res).await;
        assert!(ws["secretKeys"]["local"].as_array().unwrap().contains(&"echoedHost".into()));
    }

    #[tokio::test]
    async fn send_redacts_and_render_masks() {
        let base = echo_headers_server().await;
        let (workspace, secrets, _) = echo_workspace(&base);
        let endpoint = serde_json::to_value(&workspace.collections[0].endpoints[0]).unwrap();
        let db = require_db!();
        let app = app_with(&db, workspace, secrets).await;
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
        let db = require_db!();
        let app = app_with(&db, workspace, secrets).await;
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
        let db = require_db!();
        let app = app_with(&db, workspace, secrets).await;
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
        let stop = Request::delete(format!("/api/runs/{run_id}"))
            .header(header::HOST, HOST)
            .header(TOKEN_HEADER, TOKEN)
            .header(WORKSPACE_HEADER, WS)
            .body(Body::empty())
            .unwrap();
        assert_eq!(status(&app, stop).await, StatusCode::ACCEPTED);
    }

    /// M3 "done when": bad responses are flagged, declared 4xx aren't errors, undeclared codes are.
    #[tokio::test]
    async fn latency_run_checks_responses_against_the_contract() {
        use crate::model::{Collection, Endpoint, Expectation, ExpectedResponse, HttpMethod};
        let app = Router::new().route(
            "/pets/{id}",
            axum::routing::get(|axum::extract::Path(id): axum::extract::Path<u32>| async move {
                match id {
                    1 => (StatusCode::OK, r#"{"id":1,"name":"Rex"}"#),
                    2 => (StatusCode::OK, r#"{"id":"two","name":"Tom"}"#),
                    3 => (StatusCode::NOT_FOUND, ""),
                    _ => (StatusCode::INTERNAL_SERVER_ERROR, "boom"),
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let id = uuid::Uuid::new_v4();
        let workspace = Workspace {
            collections: vec![Collection {
                id: uuid::Uuid::new_v4(),
                name: "pets".into(),
                vars: [("base".to_string(), base)].into(),
                source: None,
                schema_defs: Some(serde_json::json!({ "components": { "schemas": { "Pet": {
                    "type": "object", "required": ["id", "name"], "properties": { "id": { "type": "integer" } }
                }}}})),
                endpoints: vec![Endpoint {
                    extract: vec![],
                    id,
                    name: "get pet".into(),
                    group: None,
                    method: HttpMethod::Get,
                    // 1, 2, 3, 4 across the four samples.
                    url: "{{base}}/pets/{{seq}}".into(),
                    headers: vec![],
                    query: vec![],
                    body: Default::default(),
                    auth: Default::default(),
                    expect: Some(Expectation {
                        responses: vec![
                            ExpectedResponse {
                                status: "200".into(),
                                schema: Some(serde_json::json!({ "$ref": "#/components/schemas/Pet" })),
                            },
                            ExpectedResponse { status: "404".into(), schema: None },
                        ],
                    }),
                }],
            }],
            ..Default::default()
        };
        let db = require_db!();
        let app = app_with(&db, workspace, Secrets::default()).await;
        let config = format!(
            r#"{{"kind":"latency","endpointId":"{id}","warmup":0,"samples":4,"keepAlive":true,"timeoutMs":5000}}"#
        );
        let StartRunResponse { run_id } =
            json_body(app.clone().oneshot(post_json("/api/runs", config)).await.unwrap()).await;
        let events = get(&format!("/api/runs/{run_id}/events")).body(Body::empty()).unwrap();
        app.clone().oneshot(events).await.unwrap().into_body().collect().await.unwrap();
        let report: RunReport = json_body(
            app.oneshot(get(&format!("/api/runs/{run_id}/report")).body(Body::empty()).unwrap()).await.unwrap(),
        )
        .await;

        let contract = report.contract.expect("contract summary");
        assert_eq!((contract.checked, contract.schema_mismatch, contract.undeclared_status), (4, 1, 1));
        assert!(contract.examples.iter().any(|m| m.contains("/id")), "{:?}", contract.examples);
        assert!(!contract.sampled);
        assert_eq!(report.total_errors, 1, "declared 404 is fine; undeclared 500 is an error");
        assert!(report.samples.iter().any(|s| s.contract.as_ref().is_some_and(|c| !c.passed)));
    }

    #[tokio::test]
    async fn complexity_run_sweeps_every_size_and_fits() {
        use crate::model::{Body as ReqBody, Collection, Endpoint, HttpMethod};
        let app = Router::new().route(
            "/count",
            axum::routing::post(|body: axum::body::Bytes| async move {
                let items: Vec<i64> = serde_json::from_slice(&body).unwrap();
                items.len().to_string()
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let (with_n, without_n) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        let endpoint = |id, body: &str| Endpoint {
            id,
            name: String::new(),
            group: None,
            method: HttpMethod::Post,
            url: "{{base}}/count".into(),
            headers: vec![],
            query: vec![],
            body: ReqBody::Json { content: body.into() },
            auth: Default::default(),
            expect: None,
            extract: vec![],
        };
        let workspace = Workspace {
            collections: vec![Collection {
                id: uuid::Uuid::new_v4(),
                name: "c".into(),
                vars: [("base".to_string(), base)].into(),
                source: None,
                schema_defs: None,
                endpoints: vec![endpoint(with_n, "{{n:int_array}}"), endpoint(without_n, "[1,2,3]")],
            }],
            ..Default::default()
        };
        let db = require_db!();
        let app = app_with(&db, workspace, Secrets::default()).await;
        let config = |id: uuid::Uuid| {
            format!(
                r#"{{"kind":"complexity","endpointId":"{id}","minN":1,"maxN":64,"points":4,"samples":3,"warmup":1,"timeoutMs":5000,"keepAlive":true,"slowMs":5000,"budgetMs":60000}}"#
            )
        };

        let res = app.clone().oneshot(post_json("/api/runs", config(without_n))).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body: serde_json::Value = json_body(res).await;
        assert!(body["error"].as_str().unwrap().contains("{{n}}"), "{body}");

        let StartRunResponse { run_id } =
            json_body(app.clone().oneshot(post_json("/api/runs", config(with_n))).await.unwrap()).await;
        let events =
            app.clone().oneshot(get(&format!("/api/runs/{run_id}/events")).body(Body::empty()).unwrap()).await.unwrap();
        let stream = String::from_utf8(events.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
        assert!(stream.contains(r#""type":"complexity""#), "progress events are streamed");

        let report: RunReport = json_body(
            app.oneshot(get(&format!("/api/runs/{run_id}/report")).body(Body::empty()).unwrap()).await.unwrap(),
        )
        .await;
        assert_eq!(report.status, RunStatus::Completed);
        assert_eq!(report.total_requests, 12, "4 sizes × 3 rounds; warm-up excluded");
        let result = report.complexity.expect("complexity result");
        let sizes: Vec<u64> = result.points.iter().map(|p| p.n).collect();
        assert_eq!(sizes, vec![1, 4, 16, 64]);
        assert!(result.points.iter().all(|p| p.samples == 3));
        assert!(result.points[3].request_bytes > result.points[0].request_bytes * 10, "bodies grow with n");
        assert!(!result.analysis.fits.is_empty());
    }

    /// Start a short fake run, read the whole SSE stream, then fetch the report.
    #[tokio::test]
    async fn fake_run_streams_to_finished_and_produces_a_report() {
        let db = require_db!();
        let app = empty_app(&db).await;
        let req = Request::post("/api/runs")
            .header(header::HOST, HOST)
            .header(TOKEN_HEADER, TOKEN)
            .header(WORKSPACE_HEADER, WS)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"kind":"fake","durationMs":600}"#))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let StartRunResponse { run_id } = json_body(res).await;

        let res =
            app.clone().oneshot(get(&format!("/api/runs/{run_id}/events")).body(Body::empty()).unwrap()).await.unwrap();
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

        let res = app.oneshot(get(&format!("/api/runs/{run_id}/report")).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let report: RunReport = json_body(res).await;
        assert_eq!(report.status, RunStatus::Completed);
        assert!(report.total_requests > 0);
    }

    #[tokio::test]
    async fn requires_a_workspace_id() {
        let db = require_db!();
        let app = empty_app(&db).await;
        let req = |ws: Option<&str>| {
            let req = Request::get("/api/workspace").header(header::HOST, HOST).header(TOKEN_HEADER, TOKEN);
            match ws {
                Some(ws) => req.header(WORKSPACE_HEADER, ws),
                None => req,
            }
            .body(Body::empty())
            .unwrap()
        };
        assert_eq!(status(&app, req(None)).await, StatusCode::BAD_REQUEST);
        assert_eq!(status(&app, req(Some("../../etc"))).await, StatusCode::BAD_REQUEST);
        assert_eq!(status(&app, req(Some(WS))).await, StatusCode::OK);
        // Health doesn't touch a workspace.
        let health = Request::get("/api/health").header(header::HOST, HOST).header(TOKEN_HEADER, TOKEN);
        assert_eq!(status(&app, health.body(Body::empty()).unwrap()).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn workspaces_dont_see_each_others_data_or_runs() {
        let db = require_db!();
        let app = empty_app(&db).await;
        let other = uuid::Uuid::new_v4().to_string();
        let as_other = |req: axum::http::request::Builder| {
            req.header(header::HOST, HOST).header(TOKEN_HEADER, TOKEN).header(WORKSPACE_HEADER, other.as_str())
        };

        let saved = r#"{"environments":[{"name":"mine","vars":{}}]}"#;
        let put = Request::put("/api/workspace")
            .header(header::HOST, HOST)
            .header(TOKEN_HEADER, TOKEN)
            .header(WORKSPACE_HEADER, WS)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(saved))
            .unwrap();
        assert_eq!(status(&app, put).await, StatusCode::NO_CONTENT);
        let res = app.clone().oneshot(as_other(Request::get("/api/workspace")).body(Body::empty()).unwrap()).await;
        let ws: serde_json::Value = json_body(res.unwrap()).await;
        assert_eq!(ws["workspace"]["environments"], serde_json::json!([]));

        let StartRunResponse { run_id } = json_body(
            app.clone().oneshot(post_json("/api/runs", r#"{"kind":"fake","durationMs":60000}"#.into())).await.unwrap(),
        )
        .await;
        let res = app.clone().oneshot(as_other(Request::get("/api/runs")).body(Body::empty()).unwrap()).await;
        let runs: Vec<RunSummary> = json_body(res.unwrap()).await;
        assert!(runs.is_empty(), "{runs:?}");
        let stop = |req: axum::http::request::Builder| req.body(Body::empty()).unwrap();
        let path = format!("/api/runs/{run_id}");
        assert_eq!(status(&app, stop(as_other(Request::delete(&path)))).await, StatusCode::NOT_FOUND);
        let events = format!("/api/runs/{run_id}/events");
        assert_eq!(status(&app, stop(as_other(Request::get(&events)))).await, StatusCode::NOT_FOUND);
        assert_eq!(status(&app, get(&path).method("DELETE").body(Body::empty()).unwrap()).await, StatusCode::ACCEPTED);
    }
}
