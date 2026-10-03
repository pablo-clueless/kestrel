use super::{
    client::Outcome,
    types::{Sample, SentRequest},
};
use crate::{redact::Redactor, template::request::RenderedRequest};

/// Builds a redacted [`Sample`], keeping at most `max_body` bytes of the response body.
pub fn build(req: &RenderedRequest, outcome: &Outcome, redactor: &Redactor, max_body: usize) -> Sample {
    let body_bytes = outcome.body.len();
    let kept = &outcome.body[..body_bytes.min(max_body)];
    Sample {
        request: SentRequest {
            method: req.method,
            url: redactor.text(req.url.as_str()),
            headers: redactor.headers(req.headers.iter().map(|(k, v)| (k.as_str(), v.as_str()))),
            body: req.body_text().map(|b| redactor.text(&b)),
        },
        status: outcome.status,
        error: outcome.error.as_ref().map(|(_, msg)| redactor.text(msg)),
        error_class: outcome.error.as_ref().map(|(class, _)| *class),
        ttfb_ms: outcome.ttfb.as_secs_f64() * 1000.0,
        total_ms: outcome.total.as_secs_f64() * 1000.0,
        connect_ms: outcome.connect.map(|d| d.as_secs_f64() * 1000.0),
        response_headers: redactor.headers(outcome.response_headers.iter().map(|(k, v)| (k.as_str(), v.as_str()))),
        body: redactor.text(&String::from_utf8_lossy(kept)),
        body_bytes: body_bytes as u64,
        body_truncated: body_bytes > max_body,
        contract: None,
    }
}
