use std::{collections::HashSet, convert::Infallible, sync::Arc};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures::Stream;
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

use super::{AppState, Scope};
use crate::{
    contract::{self, Contract},
    engine::{
        self, Prepared, complexity, concurrency, latency, load,
        registry::Envelope,
        types::{LoadMode, RunConfig, RunEvent, RunStatus, RunSummary, StartRunResponse},
    },
    error::ApiError,
    export,
    template::request::CompiledRequest,
};

pub async fn start(
    State(state): State<AppState>,
    scope: Scope,
    Json(config): Json<RunConfig>,
) -> Result<(StatusCode, Json<StartRunResponse>), ApiError> {
    let prepared = prepare(&state, &scope, &config).await?;
    let run = state.runs.create(scope.id, config);
    let run_id = run.id;
    engine::spawn(&state.runs, &scope.store, run, prepared);
    Ok((StatusCode::CREATED, Json(StartRunResponse { run_id })))
}

/// Validates the config against the caps and does everything that can fail before the run exists,
/// so mistakes come back as a 400 with a message rather than as a failed run.
async fn prepare(state: &AppState, scope: &Scope, config: &RunConfig) -> Result<Prepared, ApiError> {
    let caps = &state.config.caps;
    let in_range = |name: &str, value: u32, lo: u32, hi: u32| {
        if (lo..=hi).contains(&value) {
            Ok(())
        } else {
            Err(ApiError::BadRequest(format!(
                "{name} must be between {lo} and {hi} (caps are raised in local config, not the API)"
            )))
        }
    };
    let max_duration_ms = caps.max_duration.as_millis() as u32;
    let max_timeout_ms = caps.max_timeout.as_millis() as u32;

    match config {
        RunConfig::Fake(cfg) => {
            in_range("durationMs", cfg.duration_ms, 1, max_duration_ms)?;
            Ok(Prepared::Fake(cfg.clone()))
        }
        RunConfig::Latency(cfg) => {
            in_range("samples", cfg.samples, 1, caps.max_samples)?;
            in_range("warmup", cfg.warmup, 0, caps.max_warmup)?;
            in_range("timeoutMs", cfg.timeout_ms, 1, max_timeout_ms)?;
            let (request, contract) = compile_endpoint(scope, cfg.endpoint_id, cfg.environment.as_deref()).await?;
            let mut cfg = cfg.clone();
            cfg.ok_statuses.extend(contract::ok_statuses_from(contract.as_ref()));
            let mut prepared = latency::prepare(cfg, request, caps.max_duration, scope.store.confirmed_hosts())
                .await
                .map_err(ApiError::BadRequest)?;
            prepared.contract = contract.map(Arc::new);
            Ok(Prepared::Latency(Box::new(prepared)))
        }
        RunConfig::Load(cfg) => {
            // A breakpoint run is as long as its steps; past the cap it stops there and says so.
            let breakpoint = matches!(cfg.mode, LoadMode::Breakpoint(_) | LoadMode::RateLimit(_));
            if !breakpoint {
                in_range("durationMs", cfg.duration_ms, 1, max_duration_ms)?;
            }
            if let LoadMode::Spike(ref m) = cfg.mode {
                in_range("baseRate", m.base_rate, 1, caps.max_rps)?;
                in_range("spikeRate", m.spike_rate, m.base_rate, caps.max_rps)?;
                in_range("beforeMs", m.before_ms, 2_000, max_duration_ms)?;
                in_range("spikeMs", m.spike_ms, 1_000, max_duration_ms)?;
                in_range("afterMs", m.after_ms, 1_000, max_duration_ms)?;
                if m.total_ms() > u64::from(max_duration_ms) {
                    return Err(ApiError::BadRequest(format!(
                        "the spike run would last {} s, over the {} s duration cap (raised in local config)",
                        m.total_ms() / 1000,
                        max_duration_ms / 1000
                    )));
                }
            }
            in_range("rampUpMs", cfg.ramp_up_ms, 0, cfg.duration_ms)?;
            in_range("timeoutMs", cfg.timeout_ms, 1, max_timeout_ms)?;
            match cfg.mode {
                LoadMode::Closed { concurrency } => in_range("concurrency", concurrency, 1, caps.max_in_flight)?,
                LoadMode::Open { rate } => in_range("rate", rate, 1, caps.max_rps)?,
                // Checked above.
                LoadMode::Spike(_) => {}
                LoadMode::RateLimit(ref m) => {
                    in_range("startRate", m.start_rate, 1, caps.max_rps)?;
                    in_range("maxRate", m.max_rate, m.start_rate, caps.max_rps)?;
                    in_range("stepPercent", m.step_percent, 1, 1_000)?;
                    in_range("stepMs", m.step_ms, 1_000, max_duration_ms)?;
                }
                LoadMode::Breakpoint(ref m) => {
                    in_range("startRate", m.start_rate, 1, caps.max_rps)?;
                    in_range("maxRate", m.max_rate, m.start_rate, caps.max_rps)?;
                    in_range("stepPercent", m.step_percent, 1, 1_000)?;
                    in_range("stepMs", m.step_ms, 1_000, max_duration_ms)?;
                    if !(0.0..=100.0).contains(&m.max_error_pct) {
                        return Err(ApiError::BadRequest("maxErrorPct must be between 0 and 100".into()));
                    }
                    if let Some(p99) = m.max_p99_ms {
                        in_range("maxP99Ms", p99, 1, max_timeout_ms)?;
                    }
                }
            }
            if let Some(n) = cfg.max_in_flight {
                in_range("maxInFlight", n, 1, caps.max_in_flight)?;
            }
            let (request, contract) = compile_endpoint(scope, cfg.endpoint_id, cfg.environment.as_deref()).await?;
            let mut cfg = cfg.clone();
            cfg.ok_statuses.extend(contract::ok_statuses_from(contract.as_ref()));
            if breakpoint {
                cfg.duration_ms = cfg.duration_ms.clamp(1, max_duration_ms);
            }
            let mut prepared =
                load::prepare(cfg, request, &scope.store.confirmed_hosts(), caps.max_in_flight, caps.max_rps)
                    .await
                    .map_err(|e| match e {
                        load::PrepareError::Invalid(msg) => ApiError::BadRequest(msg),
                        load::PrepareError::HostNotConfirmed(host) => ApiError::HostNotConfirmed(host),
                    })?;
            prepared.contract = contract.map(Arc::new);
            Ok(Prepared::Load(Box::new(prepared)))
        }
        RunConfig::Complexity(cfg) => {
            in_range("minN", cfg.min_n, 1, caps.max_n)?;
            in_range("maxN", cfg.max_n, cfg.min_n, caps.max_n)?;
            in_range("points", cfg.points, 3, caps.max_points)?;
            in_range("samples", cfg.samples, 1, caps.max_samples)?;
            in_range("warmup", cfg.warmup, 0, caps.max_warmup)?;
            in_range("timeoutMs", cfg.timeout_ms, 1, max_timeout_ms)?;
            in_range("slowMs", cfg.slow_ms, 1, max_timeout_ms)?;
            in_range("budgetMs", cfg.budget_ms, 1_000, caps.max_sweep_duration.as_millis() as u32)?;
            let (request, _) = compile_endpoint(scope, cfg.endpoint_id, cfg.environment.as_deref()).await?;
            let prepared =
                complexity::prepare(cfg.clone(), request, caps.max_sweep_duration, scope.store.confirmed_hosts())
                    .await
                    .map_err(ApiError::BadRequest)?;
            Ok(Prepared::Complexity(Box::new(prepared)))
        }
        RunConfig::Concurrency(cfg) => {
            in_range("requests", cfg.requests, 2, caps.max_in_flight.min(concurrency::MAX_BURST))?;
            in_range("rounds", cfg.rounds, 1, concurrency::MAX_ROUNDS)?;
            in_range("pauseMs", cfg.pause_ms, 0, 60_000)?;
            in_range("timeoutMs", cfg.timeout_ms, 1, max_timeout_ms)?;
            let (request, _) = compile_endpoint(scope, cfg.endpoint_id, cfg.environment.as_deref()).await?;
            let prepared =
                concurrency::prepare(cfg.clone(), request, &scope.store.confirmed_hosts(), caps.max_duration)
                    .await
                    .map_err(|e| match e {
                        load::PrepareError::Invalid(msg) => ApiError::BadRequest(msg),
                        load::PrepareError::HostNotConfirmed(host) => ApiError::HostNotConfirmed(host),
                    })?;
            Ok(Prepared::Concurrency(Box::new(prepared)))
        }
    }
}

/// The request to send, and the contract to check responses against (for imported endpoints).
async fn compile_endpoint(
    scope: &Scope,
    id: Uuid,
    environment: Option<&str>,
) -> Result<(CompiledRequest, Option<Contract>), ApiError> {
    let store = &scope.store;
    let workspace = store.workspace();
    let endpoint = workspace
        .endpoint(id)
        .ok_or_else(|| ApiError::BadRequest("endpoint not found; save the workspace first".into()))?;
    let files = store.files_for(endpoint).await.map_err(|e| ApiError::Internal(format!("{e:#}")))?;
    let request = CompiledRequest::compile_with(endpoint, &workspace, &store.secrets(), &files, environment, false)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let contract = contract::for_endpoint(&workspace, endpoint).map_err(ApiError::BadRequest)?;
    Ok((request, contract))
}

/// Runs in memory (in progress or recently finished) and saved ones, newest first.
pub async fn list(State(state): State<AppState>, scope: Scope) -> Result<Json<Vec<RunSummary>>, ApiError> {
    let mut runs = state.runs.list(scope.id);
    let live: HashSet<Uuid> = runs.iter().map(|r| r.run_id).collect();
    let saved = scope.store.runs().await.map_err(|e| ApiError::Internal(format!("{e:#}")))?;
    runs.extend(saved.into_iter().filter(|r| !live.contains(&r.run_id)));
    runs.sort_by_key(|r| std::cmp::Reverse(r.started_at_ms));
    Ok(Json(runs))
}

pub async fn stop(State(state): State<AppState>, scope: Scope, Path(id): Path<Uuid>) -> Result<StatusCode, ApiError> {
    let run = state.runs.get(scope.id, id).ok_or(ApiError::RunNotFound)?;
    run.cancel.cancel();
    Ok(StatusCode::ACCEPTED)
}

#[derive(Deserialize)]
pub struct ReportQuery {
    /// `json` (default) or `csv` (see `export`).
    format: Option<String>,
}

pub async fn report(
    State(state): State<AppState>,
    scope: Scope,
    Path(id): Path<Uuid>,
    Query(query): Query<ReportQuery>,
) -> Result<Response, ApiError> {
    let report = match state.runs.get(scope.id, id) {
        Some(run) if run.status() == RunStatus::Running => return Err(ApiError::Conflict("run is still in progress")),
        Some(run) => run.report().ok_or(ApiError::Conflict("report not available yet"))?,
        // Evicted from memory (or from before a restart): serve the saved report.
        None => scope
            .store
            .run_report(id)
            .await
            .map_err(|e| ApiError::Internal(format!("{e:#}")))?
            .ok_or(ApiError::RunNotFound)?,
    };
    match query.format.as_deref() {
        None | Some("json") => Ok(Json(report).into_response()),
        Some("csv") => {
            let disposition = format!("attachment; filename=\"{}\"", export::file_name(&report, "csv"));
            let headers = [
                (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_owned()),
                (header::CONTENT_DISPOSITION, disposition),
            ];
            Ok((headers, export::csv(&report)).into_response())
        }
        Some(_) => Err(ApiError::BadRequest("format must be json or csv".into())),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsQuery {
    /// Same as the `Last-Event-ID` header, for clients that can't set it.
    last_event_id: Option<u64>,
}

/// Replays the run's history (after `Last-Event-ID`, if given), then streams live events.
/// The stream ends after `Finished`.
pub async fn events(
    State(state): State<AppState>,
    scope: Scope,
    Path(id): Path<Uuid>,
    Query(query): Query<EventsQuery>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let run = state.runs.get(scope.id, id).ok_or(ApiError::RunNotFound)?;
    let after = headers.get("last-event-id").and_then(|v| v.to_str().ok()?.parse().ok()).or(query.last_event_id);

    let stream = async_stream::stream! {
        let mut last = after.unwrap_or(0);
        let mut sub = run.subscribe(after);
        loop {
            if sub.gap {
                yield Ok(to_sse(None, &RunEvent::Resync));
            }
            for envelope in std::mem::take(&mut sub.backlog) {
                last = envelope.id;
                yield Ok(envelope_to_sse(&envelope));
                if envelope.event.is_finished() {
                    return;
                }
            }
            // Follow live until we fall behind, then resubscribe from `last` via history.
            loop {
                match sub.rx.recv().await {
                    Ok(envelope) if envelope.id <= last => continue,
                    Ok(envelope) => {
                        last = envelope.id;
                        yield Ok(envelope_to_sse(&envelope));
                        if envelope.event.is_finished() {
                            return;
                        }
                    }
                    Err(RecvError::Lagged(_)) => break,
                    Err(RecvError::Closed) => return,
                }
            }
            sub = run.subscribe(Some(last));
        }
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

fn envelope_to_sse(envelope: &Envelope) -> Event {
    to_sse(Some(envelope.id), &envelope.event)
}

fn to_sse(id: Option<u64>, event: &RunEvent) -> Event {
    let event_json = serde_json::to_string(event).expect("RunEvent serializes");
    let sse = Event::default().data(event_json);
    match id {
        Some(id) => sse.id(id.to_string()),
        None => sse,
    }
}
