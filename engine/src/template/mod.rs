//! `{{…}}` templates.
//!
//! - `{{name}}` — environment variable, then secret. Bound once when a run starts.
//! - `{{uuid}}`, `{{seq}}`, `{{int:1..1000}}` — generators, fresh on every request.
//!
//! Templates are parsed and bound once per run; per request only generators are evaluated.

pub mod request;

use std::{
    collections::BTreeMap,
    fmt::Write,
    sync::atomic::{AtomicU64, Ordering},
};

use rand::RngExt;

/// Shown in place of secret values in previews. ASCII so it survives URL encoding unchanged.
pub const MASK: &str = "******";

#[derive(Debug, Clone, PartialEq)]
enum Segment {
    Lit(String),
    Var(String),
    Gen(Generator),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Generator {
    Uuid,
    /// Per-run counter starting at 1; unique under concurrency.
    Seq,
    /// Uniform in `lo..=hi`.
    Int { lo: i64, hi: i64 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    segments: Vec<Segment>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum TemplateError {
    #[error("unclosed `{{{{` in `{0}`")]
    Unclosed(String),
    #[error("empty `{{{{}}}}` in `{0}`")]
    Empty(String),
    #[error("bad generator `{0}`: {1}")]
    BadGenerator(String, &'static str),
}

impl Template {
    pub fn parse(src: &str) -> Result<Self, TemplateError> {
        let mut segments = Vec::new();
        let mut rest = src;
        while let Some(start) = rest.find("{{") {
            if start > 0 {
                segments.push(Segment::Lit(rest[..start].to_owned()));
            }
            let after = &rest[start + 2..];
            let end = after.find("}}").ok_or_else(|| TemplateError::Unclosed(src.to_owned()))?;
            let inner = after[..end].trim();
            if inner.is_empty() {
                return Err(TemplateError::Empty(src.to_owned()));
            }
            segments.push(parse_placeholder(inner)?);
            rest = &after[end + 2..];
        }
        if !rest.is_empty() {
            segments.push(Segment::Lit(rest.to_owned()));
        }
        Ok(Self { segments })
    }

    /// Replaces variables with their values. Unknown names are appended to `missing`.
    pub fn bind(&self, scope: &Scope, missing: &mut Vec<String>) -> Bound {
        let mut out: Vec<BoundSegment> = Vec::new();
        let push_lit = |out: &mut Vec<BoundSegment>, s: &str| match out.last_mut() {
            Some(BoundSegment::Lit(prev)) => prev.push_str(s),
            _ => out.push(BoundSegment::Lit(s.to_owned())),
        };
        for seg in &self.segments {
            match seg {
                Segment::Lit(s) => push_lit(&mut out, s),
                Segment::Gen(g) => out.push(BoundSegment::Gen(*g)),
                Segment::Var(name) => match scope.lookup(name) {
                    Some(value) => push_lit(&mut out, value),
                    None => {
                        if !missing.contains(name) {
                            missing.push(name.clone());
                        }
                    }
                },
            }
        }
        Bound { segments: out }
    }
}

fn parse_placeholder(inner: &str) -> Result<Segment, TemplateError> {
    match inner {
        "uuid" => return Ok(Segment::Gen(Generator::Uuid)),
        "seq" => return Ok(Segment::Gen(Generator::Seq)),
        _ => {}
    }
    if let Some(range) = inner.strip_prefix("int:") {
        let bad = |why| TemplateError::BadGenerator(inner.to_owned(), why);
        let (lo, hi) = range.split_once("..").ok_or_else(|| bad("expected `int:LO..HI`"))?;
        let lo: i64 = lo.trim().parse().map_err(|_| bad("LO is not an integer"))?;
        let hi: i64 = hi.trim().parse().map_err(|_| bad("HI is not an integer"))?;
        if lo > hi {
            return Err(bad("LO is greater than HI"));
        }
        return Ok(Segment::Gen(Generator::Int { lo, hi }));
    }
    Ok(Segment::Var(inner.to_owned()))
}

/// Where variables come from: the environment's vars, then its secrets.
/// Lookup order: environment vars, then environment secrets, then collection vars (defaults).
pub struct Scope<'a> {
    pub vars: Option<&'a BTreeMap<String, String>>,
    pub secrets: Option<&'a BTreeMap<String, String>>,
    pub collection_vars: Option<&'a BTreeMap<String, String>>,
    /// Substitute [`MASK`] for secret values (previews).
    pub mask_secrets: bool,
}

impl Scope<'_> {
    fn lookup(&self, name: &str) -> Option<&str> {
        if let Some(v) = self.vars.and_then(|vars| vars.get(name)) {
            return Some(v);
        }
        if let Some(secret) = self.secrets.and_then(|s| s.get(name)) {
            return Some(if self.mask_secrets { MASK } else { secret });
        }
        self.collection_vars.and_then(|vars| vars.get(name)).map(String::as_str)
    }

    /// Secret values in scope, for redaction.
    pub fn secret_values(&self) -> Vec<String> {
        self.secrets.map(|s| s.values().filter(|v| !v.is_empty()).cloned().collect()).unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq)]
enum BoundSegment {
    Lit(String),
    Gen(Generator),
}

/// A template with variables resolved; only generators remain.
#[derive(Debug, Clone, PartialEq)]
pub struct Bound {
    segments: Vec<BoundSegment>,
}

/// Per-run generator state.
#[derive(Default)]
pub struct GenState {
    seq: AtomicU64,
}

impl Bound {
    pub fn render(&self, state: &GenState) -> String {
        let mut out = String::new();
        for seg in &self.segments {
            match seg {
                BoundSegment::Lit(s) => out.push_str(s),
                BoundSegment::Gen(Generator::Uuid) => {
                    let _ = write!(out, "{}", uuid::Uuid::new_v4());
                }
                BoundSegment::Gen(Generator::Seq) => {
                    let _ = write!(out, "{}", state.seq.fetch_add(1, Ordering::Relaxed) + 1);
                }
                BoundSegment::Gen(Generator::Int { lo, hi }) => {
                    let _ = write!(out, "{}", rand::rng().random_range(*lo..=*hi));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope<'a>(vars: &'a BTreeMap<String, String>, secrets: &'a BTreeMap<String, String>) -> Scope<'a> {
        Scope { vars: Some(vars), secrets: Some(secrets), collection_vars: None, mask_secrets: false }
    }

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect()
    }

    #[test]
    fn binds_vars_then_secrets() {
        let vars = map(&[("base", "http://x"), ("token", "from-vars")]);
        let secrets = map(&[("token", "from-secrets"), ("key", "k")]);
        let t = Template::parse("{{base}}/a?t={{ token }}&k={{key}}").unwrap();
        let mut missing = vec![];
        let bound = t.bind(&scope(&vars, &secrets), &mut missing);
        assert!(missing.is_empty());
        assert_eq!(bound.render(&GenState::default()), "http://x/a?t=from-vars&k=k");
    }

    #[test]
    fn reports_missing_vars_once() {
        let empty = BTreeMap::new();
        let mut missing = vec![];
        Template::parse("{{a}}{{b}}{{a}}").unwrap().bind(&scope(&empty, &empty), &mut missing);
        assert_eq!(missing, vec!["a", "b"]);
    }

    #[test]
    fn masks_secrets_for_previews() {
        let vars = BTreeMap::new();
        let secrets = map(&[("token", "hunter2")]);
        let s = Scope { vars: Some(&vars), secrets: Some(&secrets), collection_vars: None, mask_secrets: true };
        let bound = Template::parse("Bearer {{token}}").unwrap().bind(&s, &mut vec![]);
        assert_eq!(bound.render(&GenState::default()), format!("Bearer {MASK}"));
    }

    #[test]
    fn generators_are_fresh_per_render() {
        let empty = BTreeMap::new();
        let bound = Template::parse("{{seq}}-{{int:5..5}}").unwrap().bind(&scope(&empty, &empty), &mut vec![]);
        let state = GenState::default();
        assert_eq!(bound.render(&state), "1-5");
        assert_eq!(bound.render(&state), "2-5");

        let uuid = Template::parse("{{uuid}}").unwrap().bind(&scope(&empty, &empty), &mut vec![]);
        assert_ne!(uuid.render(&state), uuid.render(&state));
    }

    #[test]
    fn rejects_malformed_templates() {
        assert!(matches!(Template::parse("{{open"), Err(TemplateError::Unclosed(_))));
        assert!(matches!(Template::parse("{{ }}"), Err(TemplateError::Empty(_))));
        assert!(matches!(Template::parse("{{int:9..1}}"), Err(TemplateError::BadGenerator(..))));
        let plain = Template::parse("no {placeholders}").unwrap();
        assert_eq!(plain.bind(&scope(&BTreeMap::new(), &BTreeMap::new()), &mut vec![]).render(&GenState::default()), "no {placeholders}");
    }
}
