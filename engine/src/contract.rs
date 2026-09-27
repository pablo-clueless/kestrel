//! Contract checks: does a response match what the spec declares for its operation?
//! HANDOFF → Test catalogue → Contract check.

use jsonschema::Validator;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::model::{Endpoint, Expectation, Workspace};

/// The contract for `endpoint` (which may be an unsaved draft), using its collection's schema
/// definitions. None when the endpoint didn't come from a spec.
pub fn for_endpoint(workspace: &Workspace, endpoint: &Endpoint) -> Result<Option<Contract>, String> {
    let Some(expect) = &endpoint.expect else { return Ok(None) };
    let defs = workspace.collection_of(endpoint.id).and_then(|c| c.schema_defs.as_ref());
    Contract::compile(expect, defs)
}

/// Declared 4xx responses are part of the API, so they don't count as errors. 429 is still counted
/// as rate limiting.
pub fn ok_statuses_from(contract: Option<&Contract>) -> Vec<u16> {
    contract
        .map(|c| c.declared_statuses().into_iter().filter(|s| (400..500).contains(s) && *s != 429).collect())
        .unwrap_or_default()
}

/// A declared response, compiled once per run.
struct Entry {
    status: StatusMatch,
    label: String,
    validator: Option<Validator>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum StatusMatch {
    Exact(u16),
    /// `2XX` → 2.
    Range(u16),
    Default,
}

pub struct Contract {
    entries: Vec<Entry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ViolationKind {
    /// The spec declares no response for this status (and no `default`).
    UndeclaredStatus,
    /// The body doesn't match the declared schema, or isn't JSON when a schema is declared.
    SchemaMismatch,
}

/// The outcome of checking one response.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ContractCheck {
    pub passed: bool,
    pub kind: Option<ViolationKind>,
    pub message: String,
}

impl Contract {
    /// `defs` is the collection's `schema_defs`; its top-level keys are merged into each schema so
    /// `$ref`s like `#/components/schemas/Pet` resolve. Returns None if nothing is declared.
    pub fn compile(expect: &Expectation, defs: Option<&Value>) -> Result<Option<Self>, String> {
        let mut entries = Vec::new();
        for response in &expect.responses {
            let status = parse_status(&response.status)
                .ok_or_else(|| format!("unrecognised response status `{}` in spec", response.status))?;
            let validator = match &response.schema {
                None => None,
                Some(schema) => Some(
                    jsonschema::draft202012::new(&with_defs(schema, defs))
                        .map_err(|e| format!("response {} schema is invalid: {e}", response.status))?,
                ),
            };
            entries.push(Entry { status, label: response.status.clone(), validator });
        }
        Ok((!entries.is_empty()).then_some(Self { entries }))
    }

    /// Exact codes the spec declares. The run treats declared 4xx (except 429) as success.
    pub fn declared_statuses(&self) -> Vec<u16> {
        self.entries
            .iter()
            .filter_map(|e| match e.status {
                StatusMatch::Exact(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    pub fn check(&self, status: u16, body: &[u8]) -> ContractCheck {
        let entry = self
            .entries
            .iter()
            .find(|e| e.status == StatusMatch::Exact(status))
            .or_else(|| self.entries.iter().find(|e| e.status == StatusMatch::Range(status / 100)))
            .or_else(|| self.entries.iter().find(|e| e.status == StatusMatch::Default));
        let Some(entry) = entry else {
            return violation(ViolationKind::UndeclaredStatus, format!("status {status} isn't declared in the spec"));
        };
        let Some(validator) = &entry.validator else {
            return pass(format!("status {status} is declared ({}); no body schema", entry.label));
        };
        if body.is_empty() {
            return violation(
                ViolationKind::SchemaMismatch,
                format!("empty body, but the spec declares a schema for {}", entry.label),
            );
        }
        let instance: Value = match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(_) => {
                return violation(
                    ViolationKind::SchemaMismatch,
                    format!("body isn't JSON, but the spec declares a JSON schema for {}", entry.label),
                );
            }
        };
        match validator.validate(&instance) {
            Ok(()) => pass(format!("matches the {} response schema", entry.label)),
            Err(err) => {
                let path = err.instance_path().to_string();
                let at = if path.is_empty() { "the body".to_owned() } else { format!("`{path}`") };
                violation(ViolationKind::SchemaMismatch, format!("{} response: {err} (at {at})", entry.label))
            }
        }
    }
}

fn pass(message: String) -> ContractCheck {
    ContractCheck { passed: true, kind: None, message }
}

fn violation(kind: ViolationKind, message: String) -> ContractCheck {
    ContractCheck { passed: false, kind: Some(kind), message }
}

fn parse_status(s: &str) -> Option<StatusMatch> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("default") {
        return Some(StatusMatch::Default);
    }
    if s.len() == 3 && s[1..].eq_ignore_ascii_case("xx") {
        return s[..1].parse().ok().map(StatusMatch::Range);
    }
    s.parse().ok().map(StatusMatch::Exact)
}

fn with_defs(schema: &Value, defs: Option<&Value>) -> Value {
    let (Value::Object(schema), Some(Value::Object(defs))) = (schema, defs) else { return schema.clone() };
    let mut merged = schema.clone();
    for (k, v) in defs {
        merged.entry(k.clone()).or_insert_with(|| v.clone());
    }
    Value::Object(merged)
}

/// Tallies contract checks across a run.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ContractSummary {
    #[ts(type = "number")]
    pub checked: u64,
    #[ts(type = "number")]
    pub undeclared_status: u64,
    #[ts(type = "number")]
    pub schema_mismatch: u64,
    /// Up to 5 distinct violation messages, redacted.
    pub examples: Vec<String>,
    /// Under load only a sample of responses is validated.
    pub sampled: bool,
}

const MAX_EXAMPLES: usize = 5;

impl ContractSummary {
    pub fn add(&mut self, check: &ContractCheck, redact: impl Fn(&str) -> String) {
        self.checked += 1;
        match check.kind {
            None => return,
            Some(ViolationKind::UndeclaredStatus) => self.undeclared_status += 1,
            Some(ViolationKind::SchemaMismatch) => self.schema_mismatch += 1,
        }
        let message = redact(&check.message);
        if self.examples.len() < MAX_EXAMPLES && !self.examples.contains(&message) {
            self.examples.push(message);
        }
    }

}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::model::ExpectedResponse;

    fn contract() -> Contract {
        let defs = json!({ "components": { "schemas": {
            "Pet": { "type": "object", "required": ["id", "name"], "properties": {
                "id": { "type": "integer" },
                "name": { "type": "string" },
                "parent": { "$ref": "#/components/schemas/Pet" }
            }},
            "Error": { "type": "object", "required": ["message"], "properties": { "message": { "type": "string" } } }
        }}});
        let expect = Expectation {
            responses: vec![
                ExpectedResponse { status: "200".into(), schema: Some(json!({ "$ref": "#/components/schemas/Pet" })) },
                ExpectedResponse { status: "204".into(), schema: None },
                ExpectedResponse { status: "4XX".into(), schema: Some(json!({ "$ref": "#/components/schemas/Error" })) },
            ],
        };
        Contract::compile(&expect, Some(&defs)).unwrap().unwrap()
    }

    #[test]
    fn passes_matching_bodies_including_recursive_refs() {
        let c = contract();
        let body = br#"{"id":1,"name":"a","parent":{"id":2,"name":"b"}}"#;
        assert!(c.check(200, body).passed, "{:?}", c.check(200, body));
        assert!(c.check(204, b"").passed);
        assert!(c.check(404, br#"{"message":"nope"}"#).passed);
        assert_eq!(c.declared_statuses(), vec![200, 204]);
    }

    #[test]
    fn flags_schema_mismatches_with_a_location() {
        let c = contract();
        let check = c.check(200, br#"{"id":"one","name":"a"}"#);
        assert!(!check.passed);
        assert_eq!(check.kind, Some(ViolationKind::SchemaMismatch));
        assert!(check.message.contains("/id"), "{}", check.message);

        let nested = c.check(200, br#"{"id":1,"name":"a","parent":{"id":2}}"#);
        assert!(!nested.passed, "recursive $ref must be validated too");
        assert!(!c.check(200, b"<html>").passed);
    }

    #[test]
    fn flags_undeclared_statuses_unless_there_is_a_default() {
        let c = contract();
        let check = c.check(500, b"");
        assert_eq!(check.kind, Some(ViolationKind::UndeclaredStatus));

        let with_default = Contract::compile(
            &Expectation { responses: vec![ExpectedResponse { status: "default".into(), schema: None }] },
            None,
        )
        .unwrap()
        .unwrap();
        assert!(with_default.check(500, b"").passed);
    }

    #[test]
    fn summary_counts_and_keeps_distinct_examples() {
        let c = contract();
        let mut summary = ContractSummary::default();
        for _ in 0..3 {
            summary.add(&c.check(500, b""), str::to_owned);
        }
        summary.add(&c.check(204, b""), str::to_owned);
        assert_eq!((summary.checked, summary.undeclared_status, summary.examples.len()), (4, 3, 1));
    }
}
