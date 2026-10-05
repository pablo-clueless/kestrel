use serde_json::{Value, json};

use super::*;
use crate::{contract::Contract, model::Workspace, template::request::CompiledRequest};

const SHOP: &str = include_str!("fixtures/shop-3.0.yaml");
const NOTES: &str = include_str!("fixtures/notes-3.1.json");
const LEGACY: &str = include_str!("fixtures/legacy-2.0.json");

fn endpoint<'a>(c: &'a Collection, name: &str) -> &'a Endpoint {
    c.endpoints.iter().find(|e| e.name == name).unwrap_or_else(|| panic!("no endpoint `{name}`"))
}

fn body_json(e: &Endpoint) -> Value {
    match &e.body {
        Body::Json { content } => serde_json::from_str(content).unwrap(),
        other => panic!("expected a JSON body, got {other:?}"),
    }
}

fn contract_for(c: &Collection, e: &Endpoint) -> Contract {
    Contract::compile(e.expect.as_ref().unwrap(), c.schema_defs.as_ref()).unwrap().unwrap()
}

#[test]
fn imports_openapi_30_yaml() {
    let r = import(SHOP, None).unwrap();
    let c = &r.collection;
    assert_eq!(r.format, "OpenAPI 3.0.3");
    assert_eq!(c.name, "Pet Shop");
    assert_eq!(c.source.as_deref(), Some("OpenAPI 3.0.3 · Pet Shop 1.2.0"));
    assert_eq!(c.endpoints.len(), 5);
    // Server variables are substituted; path params become collection variables.
    assert_eq!(c.vars["base"], "https://eu.shop.example.com/v1");
    assert_eq!(c.vars["petId"], "42");

    let list = endpoint(c, "List pets");
    assert_eq!(list.group.as_deref(), Some("pets"));
    assert_eq!(list.url, "{{base}}/pets");
    let limit = list.query.iter().find(|q| q.key == "limit").unwrap();
    assert_eq!((limit.value.as_str(), limit.enabled), ("20", false), "optional params come disabled");
    let species = list.query.iter().find(|q| q.key == "species").unwrap();
    assert_eq!((species.value.as_str(), species.enabled), ("cat", true));
    assert!(matches!(list.auth, Auth::Bearer { .. }), "root security applies");

    // operationId names it; the $ref'd request body is synthesized, uuid becomes a generator.
    let create = endpoint(c, "createPet");
    assert_eq!(body_json(create), json!({ "name": "Rex", "tag": "string", "ref": "{{uuid}}" }));

    assert_eq!(endpoint(c, "Get a pet").url, "{{base}}/pets/{{petId}}");
    assert!(matches!(&endpoint(c, "Delete a pet").auth, Auth::ApiKey { name, .. } if name == "X-Shop-Key"));
    assert!(matches!(endpoint(c, "Health").auth, Auth::None), "`security: []` means no auth");
    assert!(endpoint(c, "Health").group.is_none());
}

#[test]
fn imported_endpoints_render_without_extra_setup() {
    let r = import(SHOP, None).unwrap();
    let ws = Workspace { collections: vec![r.collection.clone()], ..Default::default() };
    let secrets = [("x".to_string(), Default::default())].into();
    let get = endpoint(&r.collection, "Get a pet");
    // `token` is a secret the user adds; without an environment it's undefined, which is the point.
    let err = CompiledRequest::compile(get, &ws, &secrets, None, false).err().unwrap();
    assert!(err.to_string().contains("token"), "{err}");

    let health = endpoint(&r.collection, "Health");
    let req = CompiledRequest::compile(health, &ws, &secrets, None, false).unwrap().render().unwrap();
    assert_eq!(req.url.as_str(), "https://eu.shop.example.com/v1/health");
}

#[test]
fn contract_from_30_handles_allof_refs_nullable_and_exclusive_minimum() {
    let r = import(SHOP, None).unwrap();
    let c = &r.collection;
    let get = contract_for(c, endpoint(c, "Get a pet"));

    assert!(get.check(200, br#"{"id":1,"name":"Rex","tag":null}"#).passed, "nullable → allows null");
    let zero = get.check(200, br#"{"id":0,"name":"Rex"}"#);
    assert!(!zero.passed, "exclusiveMinimum: true with minimum 0 must reject 0");
    assert!(!get.check(200, br#"{"name":"Rex"}"#).passed, "allOf keeps `id` required");
    assert!(!get.check(200, br#"{"id":1,"name":"Rex","parent":{"id":2}}"#).passed, "recursive ref is checked");
    assert!(get.check(404, b"").passed, "declared 404 without a schema");
    assert!(!get.check(500, b"").passed, "undeclared status");

    let list = contract_for(c, endpoint(c, "List pets"));
    assert!(list.check(500, br#"{"message":"boom"}"#).passed, "`default` response covers 500");
    assert!(!list.check(500, br#"{"oops":1}"#).passed);
}

#[test]
fn imports_openapi_31_json() {
    let r = import(NOTES, Some("My Notes")).unwrap();
    let c = &r.collection;
    assert_eq!((c.name.as_str(), r.format.as_str()), ("My Notes", "OpenAPI 3.1.0"));
    assert_eq!(c.vars["base"], "/api");
    assert!(r.warnings.iter().any(|w| w.contains("relative")), "{:?}", r.warnings);

    let create = endpoint(c, "Create note");
    assert_eq!(body_json(create), json!({ "text": "hello", "pinned": false }), "named example wins");
    let contract = contract_for(c, create);
    assert!(contract.check(201, br#"{"text":"x","pinned":null}"#).passed, "2XX range + 3.1 type arrays");
    assert!(!contract.check(201, br#"{"pinned":true}"#).passed);
}

#[test]
fn imports_swagger_20() {
    let r = import(LEGACY, None).unwrap();
    let c = &r.collection;
    assert_eq!(r.format, "Swagger 2.0");
    assert_eq!(c.vars["base"], "https://orders.example.com/api", "prefers https, trims basePath slash");
    assert_eq!(c.vars["orderId"], "3fa85f64-5717-4562-b3fc-2c963f66afa6", "literal uuid, not a generator");

    let put = endpoint(c, "Update order");
    assert!(matches!(put.auth, Auth::Basic { .. }));
    assert_eq!(put.headers.len(), 1);
    assert_eq!(body_json(put), json!({ "qty": 1, "note": "string" }));
    let contract = contract_for(c, put);
    assert!(contract.check(200, br#"{"qty":2,"note":null}"#).passed, "x-nullable");
    assert!(!contract.check(200, br#"{"qty":0}"#).passed, "minimum");
}

#[test]
fn rejects_what_it_cannot_import() {
    assert!(import("", None).unwrap_err().contains("empty"));
    assert!(import("{ nope", None).unwrap_err().contains("JSON"));
    assert!(import("openapi: 2.5.0\n", None).unwrap_err().contains("isn't supported"));
    // Postman collections import (see `postman.rs`); environments are refused with a pointer.
    assert_eq!(import(r#"{"info":{"_postman_id":"x","name":"P"},"item":[]}"#, None).unwrap().format, "Postman v2.1");
    let env = r#"{"name":"Prod","values":[],"_postman_variable_scope":"environment"}"#;
    assert!(import(env, None).unwrap_err().contains("Postman environment"));
    assert!(import("title: hello\n", None).unwrap_err().contains("not an OpenAPI"));
}

#[test]
fn imports_form_bodies() {
    let spec = json!({
        "openapi": "3.0.3",
        "info": { "title": "Forms", "version": "1" },
        "paths": {
            "/login": { "post": { "summary": "login", "requestBody": { "content": {
                "application/x-www-form-urlencoded": { "schema": { "type": "object", "properties": {
                    "username": { "type": "string", "example": "ada" },
                    "password": { "type": "string", "example": "pw" }
                } } }
            } } } },
            "/upload": { "post": { "summary": "upload", "requestBody": { "content": {
                "multipart/form-data": { "schema": { "type": "object", "properties": {
                    "title": { "type": "string", "example": "cat" },
                    "file": { "type": "string", "format": "binary" }
                } } }
            } } } }
        }
    });
    let r = import(&spec.to_string(), None).unwrap();
    let login = endpoint(&r.collection, "login");
    let Body::Form { fields } = &login.body else { panic!("expected a form, got {:?}", login.body) };
    let pairs: Vec<_> = fields.iter().map(|f| format!("{}={}", f.key, f.value)).collect();
    assert_eq!(pairs, ["password=pw", "username=ada"]);

    let upload = endpoint(&r.collection, "upload");
    let Body::Multipart { fields } = &upload.body else { panic!("expected multipart, got {:?}", upload.body) };
    let parts: Vec<_> = fields.iter().map(|f| (f.key.as_str(), f.kind, f.value.as_str())).collect();
    assert_eq!(parts, [("title", FieldKind::Text, "cat"), ("file", FieldKind::File, "")]);
    assert!(fields[1].file.is_none(), "files are chosen by hand");
    assert!(r.warnings.iter().any(|w| w.contains("choose a file")), "{:?}", r.warnings);
}

#[test]
fn imports_swagger_form_data() {
    let spec = json!({
        "swagger": "2.0",
        "info": { "title": "Legacy forms", "version": "1" },
        "host": "api.example.com",
        "paths": { "/pets": { "post": {
            "summary": "add",
            "consumes": ["multipart/form-data"],
            "parameters": [
                { "in": "formData", "name": "name", "type": "string", "required": true, "x-example": "Rex" },
                { "in": "formData", "name": "photo", "type": "file" }
            ]
        } } }
    });
    let r = import(&spec.to_string(), None).unwrap();
    let add = endpoint(&r.collection, "add");
    let Body::Multipart { fields } = &add.body else { panic!("expected multipart, got {:?}", add.body) };
    let parts: Vec<_> = fields.iter().map(|f| (f.key.as_str(), f.kind)).collect();
    assert_eq!(parts, [("name", FieldKind::Text), ("photo", FieldKind::File)]);
}

/// A spec whose schemas live in another file: bundled, then imported as if they were local, with
/// examples built from them and contract checks that follow refs into the bundled file.
#[tokio::test]
async fn external_refs_are_bundled_and_followed() {
    let mut root = json!({
        "openapi": "3.0.3",
        "info": { "title": "Split", "version": "1" },
        "servers": [{ "url": "https://api.split.io" }],
        "paths": { "/pets": { "post": {
            "operationId": "add",
            "requestBody": { "content": { "application/json": { "schema": { "$ref": "models.json#/Pet" } } } },
            "responses": { "201": { "description": "made", "content": { "application/json": {
                "schema": { "$ref": "models.json#/Pet" } } } } }
        } } }
    });
    let models = r##"{ "Pet": { "type": "object", "required": ["name"],
        "properties": { "name": { "type": "string", "example": "Rex" }, "owner": { "$ref": "#/Owner" } } },
        "Owner": { "type": "object", "properties": { "id": { "type": "integer" } } } }"##;
    let base = url::Url::parse("https://specs.split.io/v1/openapi.json").unwrap();
    let warnings = external::bundle(&mut root, Some(&base), |u: url::Url| {
        let found = (u.as_str() == "https://specs.split.io/v1/models.json").then(|| models.to_owned());
        async move { found.ok_or_else(|| "404".to_owned()) }
    })
    .await;
    assert!(warnings.is_empty(), "{warnings:?}");

    let r = import(&root.to_string(), None).unwrap();
    let c = &r.collection;
    let add = endpoint(c, "add");
    assert_eq!(body_json(add)["name"], "Rex", "the example comes from the bundled schema");
    let contract = contract_for(c, add);
    assert!(contract.check(201, br#"{"name":"Rex","owner":{"id":1}}"#).passed);
    let bad = contract.check(201, br#"{"owner":{"id":"one"}}"#);
    assert!(!bad.passed, "refs into the bundled file are checked too: {}", bad.message);
}

#[test]
fn cookie_parameters_and_cookie_api_keys_become_a_cookie_header() {
    let spec = json!({
        "openapi": "3.0.3",
        "info": { "title": "Cookies", "version": "1" },
        "servers": [{ "url": "https://c.io" }],
        "components": { "securitySchemes": { "session": { "type": "apiKey", "in": "cookie", "name": "sid" } } },
        "security": [{ "session": [] }],
        "paths": { "/me": { "get": {
            "operationId": "me",
            "parameters": [
                { "name": "lang", "in": "cookie", "required": true, "example": "en" },
                { "name": "theme", "in": "cookie", "example": "dark" }
            ],
            "responses": { "200": { "description": "ok" } }
        } } }
    });
    let r = import(&spec.to_string(), None).unwrap();
    let me = endpoint(&r.collection, "me");
    let cookie = me.headers.iter().find(|h| h.key == "Cookie").expect("a Cookie header");
    assert_eq!(cookie.value, "lang=en; sid={{apiKey}}", "required ones, and the API key");
    assert!(cookie.enabled);
    assert!(matches!(me.auth, Auth::None), "the key travels in the cookie, not as auth");
    assert!(!r.warnings.iter().any(|w| w.contains("Cookie")), "{:?}", r.warnings);
}
