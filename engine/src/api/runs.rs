use std::convert::Infallible;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::sse::{Event, KeepAlive, Sse},
};
use futures::Stream;
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

use super::AppState;
use crate::{
    engine::{
        self, Prepared, latency, load,
        registry::Envelope,
        types::{LoadMode, RunConfig, RunEvent, RunReport, RunStatus, RunSummary, StartRunResponse},
    },
    error::ApiError,
    template::request::CompiledRequest,
};

pub async fn start(
    State(state): State<AppState>,
    Json(config): Json<RunConfig>,
) -> Result<(StatusCode, Json<StartRunResponse>), ApiError> {
    let prepared = prepare(&state, &config).await?;
    let run = state.runs.create(config);
    let run_id = run.id;
    engine::spawn(&state.runs, run, prepared);
    Ok((StatusCode::CREATED, Json(StartRunResponse { run_id })))
}

/// Validates the config against the caps and does everything that can fail before the run exists,
/// so mistakes come back as a 400 with a message rather than as a failed run.
async fn prepare(state: &AppState, config: &RunConfig) -> Result<Prepared, ApiError> {
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
            let request = compile_endpoint(state, cfg.endpoint_id, cfg.environment.as_deref())?;
            let prepared = latency::prepare(cfg.clone(), request, caps.max_duration, state.hosts.list())
                .await
                .map_err(ApiError::BadRequest)?;
            Ok(Prepared::Latency(Box::new(prepared)))
        }
        RunConfig::Load(cfg) => {
            in_range("durationMs", cfg.duration_ms, 1, max_duration_ms)?;
            in_range("rampUpMs", cfg.ramp_up_ms, 0, cfg.duration_ms)?;
            in_range("timeoutMs", cfg.timeout_ms, 1, max_timeout_ms)?;
            match cfg.mode {
                LoadMode::Closed { concurrency } => in_range("concurrency", concurrency, 1, caps.max_in_flight)?,
                LoadMode::Open { rate } => in_range("rate", rate, 1, caps.max_rps)?,
            }
            if let Some(n) = cfg.max_in_flight {
                in_range("maxInFlight", n, 1, caps.max_in_flight)?;
            }
            let request = compile_endpoint(state, cfg.endpoint_id, cfg.environment.as_deref())?;
            let prepared = load::prepare(cfg.clone(), request, &state.hosts.list(), caps.max_in_flight, caps.max_rps)
                .await
                .map_err(|e| match e {
                    load::PrepareError::Invalid(msg) => ApiError::BadRequest(msg),
                    load::PrepareError::HostNotConfirmed(host) => ApiError::HostNotConfirmed(host),
                })?;
            Ok(Prepared::Load(Box::new(prepared)))
        }
    }
}

fn compile_endpoint(state: &AppState, id: Uuid, environment: Option<&str>) -> Result<CompiledRequest, ApiError> {
    let workspace = state.store.workspace();
    let endpoint = workspace
        .endpoint(id)
        .ok_or_else(|| ApiError::BadRequest("endpoint not found; save the workspace first".into()))?;
    CompiledRequest::compile(endpoint, &workspace, &state.store.secrets(), environment, false)
        .map_err(|e| ApiError::BadRequest(e.to_string()))
}

pub async fn list(State(state): State<AppState>) -> Json<Vec<RunSummary>> {
    Json(state.runs.list())
}

pub async fn stop(State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<StatusCode, ApiError> {
    let run = state.runs.get(id).ok_or(ApiError::RunNotFound)?;
    run.cancel.cancel();
    Ok(StatusCode::ACCEPTED)
}

pub async fn report(State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<RunReport>, ApiError> {
    let run = state.runs.get(id).ok_or(ApiError::RunNotFound)?;
    if run.status() == RunStatus::Running {
        return Err(ApiError::Conflict("run is still in progress"));
    }
    run.report().map(Json).ok_or(ApiError::Conflict("report not available yet"))
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
    Path(id): Path<Uuid>,
    Query(query): Query<EventsQuery>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let run = state.runs.get(id).ok_or(ApiError::RunNotFound)?;
    let after = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok()?.parse().ok())
        .or(query.last_event_id);

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
