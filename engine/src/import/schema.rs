//! Schema helpers for import: `$ref` resolution, OpenAPI → JSON Schema conversion, examples.

use serde_json::{Map, Value, json};

const MAX_REF_HOPS: usize = 32;
const MAX_DEPTH: usize = 6;

/// Follows a chain of local `$ref`s (`#/a/b`). Returns the input if it isn't a ref or can't be
/// resolved.
pub fn resolve<'a>(root: &'a Value, mut v: &'a Value) -> &'a Value {
    for _ in 0..MAX_REF_HOPS {
        let Some(r) = v.get("$ref").and_then(Value::as_str) else { return v };
        match r.strip_prefix('#').and_then(|p| root.pointer(p)) {
            Some(target) => v = target,
            None => return v,
        }
    }
    v
}

/// OpenAPI 3.0 / Swagger 2.0 schemas are a JSON Schema dialect with a few incompatible keywords.
/// Rewrites them so a draft 2020-12 validator reads them the way the spec meant. 3.1 is already
/// 2020-12 and is returned unchanged.
pub fn to_json_schema(schema: &Value, is_31: bool) -> Value {
    if is_31 {
        return schema.clone();
    }
    convert(schema)
}

fn convert(v: &Value) -> Value {
    match v {
        Value::Array(items) => Value::Array(items.iter().map(convert).collect()),
        Value::Object(obj) => {
            let mut out: Map<String, Value> = obj.iter().map(|(k, v)| (k.clone(), convert(v))).collect();
            // `nullable: true` (3.0) / `x-nullable: true` (2.0) → add "null" to `type`.
            let nullable = [out.remove("nullable"), out.remove("x-nullable")]
                .into_iter()
                .flatten()
                .any(|n| n.as_bool() == Some(true));
            if nullable {
                match out.get("type").cloned() {
                    Some(Value::String(t)) => {
                        out.insert("type".into(), json!([t, "null"]));
                    }
                    // A nullable `$ref` or composition: allow null alongside it.
                    _ if !out.contains_key("type") => {
                        let inner = Value::Object(std::mem::take(&mut out));
                        out.insert("anyOf".into(), json!([inner, { "type": "null" }]));
                    }
                    _ => {}
                }
            }
            // Boolean exclusiveMinimum/Maximum (draft 4) → numeric (2020-12).
            for (excl, bound) in [("exclusiveMinimum", "minimum"), ("exclusiveMaximum", "maximum")] {
                if let Some(Value::Bool(b)) = out.get(excl).cloned() {
                    out.remove(excl);
                    if b && let Some(n) = out.remove(bound) {
                        out.insert(excl.into(), n);
                    }
                }
            }
            // Swagger's `type: file` has no JSON Schema meaning.
            if out.get("type").and_then(Value::as_str) == Some("file") {
                out.remove("type");
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

/// A plausible value for `schema`: its example/default/enum if it has one, otherwise one built from
/// its type. With `generators`, formats like uuid become `{{uuid}}` so every request is fresh.
pub fn synthesize(root: &Value, schema: &Value, generators: bool) -> Value {
    synth(root, schema, generators, 0, &mut Vec::new())
}

fn synth(root: &Value, schema: &Value, generators: bool, depth: usize, seen: &mut Vec<String>) -> Value {
    // Recursive refs (a Pet with a parent Pet) stop at the second visit.
    let reference = schema.get("$ref").and_then(Value::as_str).map(str::to_owned);
    if let Some(r) = &reference {
        if seen.contains(r) || depth > MAX_DEPTH {
            return Value::Null;
        }
        seen.push(r.clone());
    }
    let s = resolve(root, schema);
    let out = synth_resolved(root, s, generators, depth, seen);
    if reference.is_some() {
        seen.pop();
    }
    out
}

fn synth_resolved(root: &Value, s: &Value, generators: bool, depth: usize, seen: &mut Vec<String>) -> Value {
    for key in ["example", "default", "const"] {
        if let Some(v) = s.get(key) {
            return v.clone();
        }
    }
    if let Some(v) = s.get("examples").and_then(Value::as_array).and_then(|a| a.first()) {
        return v.clone();
    }
    if let Some(v) = s.get("enum").and_then(Value::as_array).and_then(|a| a.first()) {
        return v.clone();
    }
    if let Some(parts) = s.get("allOf").and_then(Value::as_array) {
        let mut merged = Map::new();
        for part in parts {
            if let Value::Object(o) = synth(root, part, generators, depth + 1, seen) {
                merged.extend(o);
            }
        }
        return Value::Object(merged);
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(first) = s.get(key).and_then(Value::as_array).and_then(|a| a.first()) {
            return synth(root, first, generators, depth + 1, seen);
        }
    }

    let ty = match s.get("type") {
        Some(Value::String(t)) => Some(t.as_str()),
        // 3.1: `type: [string, "null"]` → the first non-null.
        Some(Value::Array(ts)) => ts.iter().filter_map(Value::as_str).find(|t| *t != "null"),
        _ => None,
    };
    let ty = ty.or_else(|| s.get("properties").map(|_| "object")).or_else(|| s.get("items").map(|_| "array"));

    match ty {
        Some("object") => {
            if depth > MAX_DEPTH {
                return json!({});
            }
            let props = s.get("properties").and_then(Value::as_object);
            Value::Object(
                props
                    .into_iter()
                    .flatten()
                    .map(|(k, v)| (k.clone(), synth(root, v, generators, depth + 1, seen)))
                    .collect(),
            )
        }
        Some("array") => match s.get("items") {
            Some(items) if depth <= MAX_DEPTH => json!([synth(root, items, generators, depth + 1, seen)]),
            _ => json!([]),
        },
        Some("integer") => s.get("minimum").cloned().unwrap_or(json!(1)),
        Some("number") => s.get("minimum").cloned().unwrap_or(json!(1.5)),
        Some("boolean") => json!(true),
        Some("null") => Value::Null,
        Some("string") | None => json!(match s.get("format").and_then(Value::as_str) {
            Some("uuid") if generators => "{{uuid}}",
            Some("uuid") => "3fa85f64-5717-4562-b3fc-2c963f66afa6",
            Some("date-time") => "2024-01-01T00:00:00Z",
            Some("date") => "2024-01-01",
            Some("email") => "user@example.com",
            Some("uri" | "url") => "https://example.com",
            Some("ipv4") => "192.0.2.1",
            _ => "string",
        }),
        Some(_) => Value::Null,
    }
}

/// Examples as the text that goes into a URL, header or form field.
pub fn example_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}
