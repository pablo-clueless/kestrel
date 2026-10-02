//! curl command → endpoint(s). Pasting a curl into a request's URL fills that request in; pasting
//! one or more into the import dialog makes a collection (e.g. DevTools' "Copy all as cURL").
//!
//! Reads what Chrome, Firefox and API docs produce: bash quoting (`'…'`, `"…"`, `$'…'`, `\`
//! continuations) and Windows `cmd` quoting (`^"…^"`, `^` continuations). It's a reader for curl's
//! request options, not a shell: variables aren't expanded, and anything that reads a file (`@file`)
//! is reported rather than read, since the file is on the user's machine, not the engine's.

use std::collections::BTreeMap;

use base64::Engine;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use url::Url;
use uuid::Uuid;

use super::ImportResult;
use crate::model::{Auth, Body, Collection, Endpoint, FieldKind, FormField, HttpMethod, KeyValue};

/// `POST /api/import/curl`: one command, to fill in a request.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CurlParseRequest {
    pub command: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CurlParseResult {
    /// The URL is as written (not `{{base}}`-relative); the UI decides how it fits the collection.
    pub endpoint: Endpoint,
    /// Options that were skipped or guessed, for the user to review.
    pub warnings: Vec<String>,
}

/// Whether `text` starts with a curl command (after blank lines and comments), so the import
/// dialog can tell it from a spec.
pub fn looks_like_curl(text: &str) -> bool {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#'));
    first.is_some_and(|l| {
        let word = l.split(|c: char| c.is_whitespace() || c == '^').next().unwrap_or("");
        word.eq_ignore_ascii_case("curl") || word.eq_ignore_ascii_case("curl.exe")
    })
}

/// The first curl command in `text`, as an endpoint.
pub fn parse_one(text: &str) -> Result<CurlParseResult, String> {
    let commands = curl_commands(text)?;
    let total = commands.len();
    let mut parsed = parse_command(&commands[0])?;
    if total > 1 {
        parsed.warnings.push(format!("Only the first of {total} curl commands was used."));
    }
    Ok(parsed)
}

/// Every curl command in `text`, as a collection. When they all go to one origin, it becomes the
/// collection's `base` variable and the URLs are `{{base}}/…`.
pub fn import(text: &str, name: Option<&str>) -> Result<ImportResult, String> {
    let mut endpoints = Vec::new();
    let mut warnings = Vec::new();
    for (i, words) in curl_commands(text)?.iter().enumerate() {
        let parsed = parse_command(words).map_err(|e| format!("command {}: {e}", i + 1))?;
        let label = parsed.endpoint.name.clone();
        warnings.extend(parsed.warnings.into_iter().map(|w| format!("{label}: {w}")));
        endpoints.push(parsed.endpoint);
    }

    let origins: Vec<Option<String>> = endpoints.iter().map(|e| origin_of(&e.url)).collect();
    let shared = origins.first().cloned().flatten().filter(|o| origins.iter().all(|x| x.as_deref() == Some(o)));
    let mut vars = BTreeMap::new();
    if let Some(origin) = &shared {
        for e in &mut endpoints {
            e.url = format!("{{{{base}}}}{}", &e.url[origin.len()..]);
        }
        vars.insert("base".to_owned(), origin.clone());
    }

    let host = shared.as_deref().and_then(|o| Url::parse(o).ok()).and_then(|u| u.host_str().map(str::to_owned));
    let count = endpoints.len();
    let collection = Collection {
        id: Uuid::new_v4(),
        name: name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
            .or(host)
            .unwrap_or_else(|| "curl import".into()),
        vars,
        endpoints,
        groups: Vec::new(),
        source: Some(format!("curl · {count} request{}", if count == 1 { "" } else { "s" })),
        schema_defs: None,
    };
    warnings.dedup();
    Ok(ImportResult { collection, format: "curl".into(), warnings })
}

/// `scheme://host[:port]` of a URL, if it has one.
fn origin_of(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    parsed.host_str()?;
    let origin = parsed.origin().ascii_serialization();
    url.starts_with(&origin).then_some(origin)
}

// ---- Splitting text into commands and words ----

/// The curl commands in `text`, each as its words (without `curl`). Other commands (`export`, a
/// prompt's `$`) are skipped.
fn curl_commands(text: &str) -> Result<Vec<Vec<String>>, String> {
    let text = if is_cmd_style(text) { from_cmd(text) } else { text.replace("\r\n", "\n") };
    let commands: Vec<Vec<String>> = split(&text)?
        .into_iter()
        .filter_map(|mut words| {
            // A pasted prompt (`$ curl …`).
            if words.first().is_some_and(|w| w == "$") {
                words.remove(0);
            }
            let first = words.first()?;
            (first.eq_ignore_ascii_case("curl") || first.eq_ignore_ascii_case("curl.exe")).then(|| words.split_off(1))
        })
        .collect();
    if commands.is_empty() {
        return Err("no curl command found; paste something that starts with `curl`".into());
    }
    Ok(commands)
}

/// Windows `cmd` quoting, as in Chrome's "Copy as cURL (cmd)": `^` escapes the next character and
/// a `^` at the end of a line continues it.
fn is_cmd_style(text: &str) -> bool {
    text.contains("^\"") || text.contains("^\n") || text.contains("^\r\n")
}

/// Undoes `cmd` escaping, leaving double quotes and `\"` for the normal splitter.
fn from_cmd(text: &str) -> String {
    let text = text.replace("^\r\n", "").replace("^\n", "").replace("\r\n", "\n");
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '^' => out.extend(chars.next()),
            _ => out.push(c),
        }
    }
    out
}

/// Shell-style word splitting: quotes, escapes, comments, and `;` / `&&` / `|` / newlines between
/// commands.
fn split(text: &str) -> Result<Vec<Vec<String>>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut commands = vec![Vec::new()];
    let mut word = String::new();
    let mut in_word = false;
    let mut i = 0;

    let end_word = |word: &mut String, in_word: &mut bool, commands: &mut Vec<Vec<String>>| {
        if *in_word {
            commands.last_mut().unwrap().push(std::mem::take(word));
            *in_word = false;
        }
    };

    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' if chars.get(i + 1) == Some(&'\n') => i += 2,
            '\\' => {
                if let Some(&next) = chars.get(i + 1) {
                    word.push(next);
                }
                in_word = true;
                i += 2;
            }
            ' ' | '\t' | '\r' => {
                end_word(&mut word, &mut in_word, &mut commands);
                i += 1;
            }
            // `a=1&b=2` in an unquoted URL stays one word; `&&`, or `&` on its own, ends a command.
            '&' if in_word && chars.get(i + 1).is_some_and(|n| *n != '&' && !n.is_whitespace()) => {
                word.push('&');
                i += 1;
            }
            '\n' | ';' | '&' | '|' => {
                end_word(&mut word, &mut in_word, &mut commands);
                if !commands.last().unwrap().is_empty() {
                    commands.push(Vec::new());
                }
                i += 1;
            }
            '#' if !in_word => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '\'' => {
                let close = find(&chars, i + 1, '\'').ok_or("a single quote (') isn't closed")?;
                word.extend(&chars[i + 1..close]);
                in_word = true;
                i = close + 1;
            }
            '$' if chars.get(i + 1) == Some(&'\'') => {
                i = ansi_c(&chars, i + 2, &mut word)?;
                in_word = true;
            }
            '"' => {
                i += 1;
                loop {
                    match chars.get(i) {
                        None => return Err("a double quote (\") isn't closed".into()),
                        Some('"') => break,
                        Some('\\') if matches!(chars.get(i + 1), Some('"' | '\\' | '$' | '`')) => {
                            word.push(chars[i + 1]);
                            i += 2;
                        }
                        Some('\\') if chars.get(i + 1) == Some(&'\n') => i += 2,
                        Some(&c) => {
                            word.push(c);
                            i += 1;
                        }
                    }
                }
                in_word = true;
                i += 1;
            }
            _ => {
                word.push(c);
                in_word = true;
                i += 1;
            }
        }
    }
    end_word(&mut word, &mut in_word, &mut commands);
    commands.retain(|c| !c.is_empty());
    Ok(commands)
}

fn find(chars: &[char], from: usize, target: char) -> Option<usize> {
    (from..chars.len()).find(|&j| chars[j] == target)
}

/// `$'…'` (ANSI-C quoting, which Chrome uses for bodies with special characters), from just after
/// the opening quote. Returns the index after the closing quote.
fn ansi_c(chars: &[char], mut i: usize, word: &mut String) -> Result<usize, String> {
    let hex = |s: &[char]| u32::from_str_radix(&s.iter().collect::<String>(), 16).ok().and_then(char::from_u32);
    loop {
        match chars.get(i) {
            None => return Err("a $'…' string isn't closed".into()),
            Some('\'') => return Ok(i + 1),
            Some('\\') => {
                let Some(&e) = chars.get(i + 1) else { return Err("a $'…' string ends with \\".into()) };
                i += 2;
                match e {
                    'n' => word.push('\n'),
                    't' => word.push('\t'),
                    'r' => word.push('\r'),
                    '0' => word.push('\0'),
                    'x' | 'u' | 'U' => {
                        let max = match e {
                            'x' => 2,
                            'u' => 4,
                            _ => 8,
                        };
                        let len = (i..chars.len().min(i + max)).take_while(|&j| chars[j].is_ascii_hexdigit()).count();
                        match hex(&chars[i..i + len]) {
                            Some(c) if len > 0 => word.push(c),
                            _ => {
                                word.push('\\');
                                word.push(e);
                            }
                        }
                        i += len;
                    }
                    other => word.push(other),
                }
            }
            Some(&c) => {
                word.push(c);
                i += 1;
            }
        }
    }
}

// ---- Options ----

/// Short options that take a value. Every other short option is a flag.
const SHORT_WITH_VALUE: &str = "AbcCdDeEFHKmoPQrtTuUwxXyYz";

/// Long options that take a value and don't affect the request, so their value is skipped.
const IGNORED_WITH_VALUE: &[&str] = &[
    "output",
    "max-time",
    "connect-timeout",
    "retry",
    "retry-delay",
    "retry-max-time",
    "proxy",
    "proxy-user",
    "write-out",
    "cacert",
    "capath",
    "cert",
    "key",
    "cert-type",
    "key-type",
    "pass",
    "resolve",
    "connect-to",
    "limit-rate",
    "cookie-jar",
    "dump-header",
    "interface",
    "dns-servers",
    "max-filesize",
    "keepalive-time",
    "expect100-timeout",
    "unix-socket",
    "abstract-unix-socket",
    "trace",
    "trace-ascii",
    "ciphers",
    "tls-max",
    "max-redirs",
    "speed-limit",
    "speed-time",
    "time-cond",
    "continue-at",
    "local-port",
    "stderr",
    "range",
    "output-dir",
    "create-file-mode",
    "happy-eyeballs-timeout-ms",
    "proto",
    "proto-redir",
    "proto-default",
    "noproxy",
    "preproxy",
    "socks5",
    "socks5-hostname",
    "socks4",
    "socks4a",
    "netrc-file",
    "engine",
];

/// Long options that matter, mapped to the short letter they share handling with (or a private
/// one for those without).
fn long_option(name: &str) -> Option<char> {
    Some(match name {
        "url" => '@',
        "request" => 'X',
        "header" => 'H',
        "data" | "data-ascii" => 'd',
        "data-raw" => 'R',
        "data-binary" => 'B',
        "data-urlencode" => 'L',
        "json" => 'J',
        "form" => 'F',
        "form-string" => 'S',
        "cookie" => 'b',
        "user" => 'u',
        "user-agent" => 'A',
        "referer" => 'e',
        "upload-file" => 'T',
        "oauth2-bearer" => 'O',
        "url-query" => 'q',
        "config" => 'K',
        "get" => 'G',
        "head" => 'I',
        _ => return None,
    })
}

/// Long flags that don't matter for the request; skipped without a warning.
const QUIET_FLAGS: &[&str] = &[
    "compressed",
    "location",
    "location-trusted",
    "insecure",
    "silent",
    "show-error",
    "verbose",
    "include",
    "fail",
    "fail-with-body",
    "globoff",
    "no-buffer",
    "path-as-is",
    "http1.0",
    "http1.1",
    "http2",
    "http2-prior-knowledge",
    "http3",
    "http3-only",
    "ipv4",
    "ipv6",
    "tlsv1",
    "tlsv1.0",
    "tlsv1.1",
    "tlsv1.2",
    "tlsv1.3",
    "sslv3",
    "progress-bar",
    "no-progress-meter",
    "raw",
    "tcp-nodelay",
    "tcp-fastopen",
    "no-keepalive",
    "no-sessionid",
    "remote-name",
    "remote-name-all",
    "remote-time",
    "create-dirs",
    "fail-early",
    "styled-output",
    "no-styled-output",
    "disable",
    "anyauth",
    "basic",
    "digest",
    "ntlm",
    "negotiate",
    "retry-connrefused",
    "retry-all-errors",
    "suppress-connect-headers",
    "parallel",
    "ssl-reqd",
    "ssl",
    "false-start",
    "cert-status",
    "no-alpn",
    "no-npn",
    "netrc",
    "netrc-optional",
];

/// Request data from `-d` and friends, before deciding the body type.
enum Data {
    Text(String),
    /// `--data-urlencode`, already encoded.
    Encoded(String),
}

#[derive(Default)]
struct Parsed {
    url: Option<String>,
    method: Option<String>,
    headers: Vec<(String, String)>,
    data: Vec<Data>,
    json: Vec<String>,
    form: Vec<(String, String, bool)>,
    user: Option<String>,
    bearer: Option<String>,
    url_query: Vec<String>,
    get: bool,
    head: bool,
    warnings: Vec<String>,
}

fn parse_command(words: &[String]) -> Result<CurlParseResult, String> {
    let mut p = Parsed::default();
    let mut i = 0;
    let mut positional_only = false;
    while i < words.len() {
        let word = &words[i];
        i += 1;
        if positional_only || !word.starts_with('-') || word == "-" {
            p.positional(word);
            continue;
        }
        if word == "--" {
            positional_only = true;
            continue;
        }
        if let Some(name) = word.strip_prefix("--") {
            // `--no-foo` turns a flag off; nothing here depends on that.
            if let Some(key) = long_option(name) {
                if matches!(key, 'G' | 'I') {
                    p.option(key, String::new());
                    continue;
                }
                let value = words.get(i).ok_or_else(|| format!("--{name} needs a value"))?.clone();
                i += 1;
                p.option(key, value);
            } else if IGNORED_WITH_VALUE.contains(&name) {
                i += 1;
            } else if !QUIET_FLAGS.contains(&name) && !name.starts_with("no-") {
                p.warnings.push(format!("Ignored --{name}."));
            }
            continue;
        }
        // Short options, possibly grouped (`-sSL`) or with the value attached (`-XPOST`).
        for (at, letter) in word[1..].char_indices() {
            if SHORT_WITH_VALUE.contains(letter) {
                let attached = &word[1 + at + letter.len_utf8()..];
                let value = if attached.is_empty() {
                    let v = words.get(i).ok_or_else(|| format!("-{letter} needs a value"))?.clone();
                    i += 1;
                    v
                } else {
                    attached.to_owned()
                };
                p.option(letter, value);
                break;
            }
            // Of the flags, only these change the request. (The others, like `-S` and `-L`, must
            // not reach `option`, which also uses letters as keys for long options.)
            if matches!(letter, 'G' | 'I') {
                p.option(letter, String::new());
            }
        }
    }
    p.finish()
}

impl Parsed {
    fn positional(&mut self, word: &str) {
        if self.url.is_none() {
            self.url = Some(word.to_owned());
        } else {
            self.warnings.push(format!("Ignored the extra argument `{word}`."));
        }
    }

    fn option(&mut self, key: char, value: String) {
        match key {
            '@' => self.url = Some(value),
            'X' => self.method = Some(value),
            'H' => self.header(&value),
            'A' => self.headers.push(("User-Agent".into(), value)),
            'e' => self.headers.push(("Referer".into(), value)),
            'b' if value.contains('=') => self.headers.push(("Cookie".into(), value)),
            'b' => self.warnings.push(format!("Skipped the cookie file `{value}`; paste the cookies instead.")),
            'd' | 'B' if value.starts_with('@') => self.file(&value[1..]),
            'd' | 'R' | 'B' => self.data.push(Data::Text(value)),
            'L' => self.data_urlencode(&value),
            'J' if value.starts_with('@') => self.file(&value[1..]),
            'J' => self.json.push(value),
            'F' | 'S' => self.form_field(&value, key == 'F'),
            'u' => self.user = Some(value),
            'O' => self.bearer = Some(value),
            'q' => self.url_query.push(value),
            'G' => self.get = true,
            'I' => self.head = true,
            'T' => self.warnings.push(format!("Skipped uploading `{value}`; choose the file in the Body tab.")),
            'K' => self.warnings.push(format!("Skipped the config file `{value}`.")),
            // Flags (`-s`, `-L`, `-k`, …) and options that don't change the request (`-o`, `-m`).
            _ => {}
        }
    }

    fn header(&mut self, raw: &str) {
        if let Some(path) = raw.strip_prefix('@') {
            self.warnings.push(format!("Skipped the header file `{path}`."));
            return;
        }
        // `Name;` sends an empty header; `Name:` removes one of curl's own, which is nothing here.
        if let Some(name) = raw.strip_suffix(';').filter(|n| !n.contains(':')) {
            self.headers.push((name.trim().to_owned(), String::new()));
        } else if let Some((name, value)) = raw.split_once(':') {
            let value = value.trim();
            if !value.is_empty() {
                self.headers.push((name.trim().to_owned(), value.to_owned()));
            }
        } else {
            self.warnings.push(format!("Skipped the header `{raw}`, which has no `:`."));
        }
    }

    fn file(&mut self, path: &str) {
        self.warnings.push(format!(
            "The body was read from the file `{path}`, which can't be imported; add it in the Body tab."
        ));
    }

    /// `--data-urlencode`: `content`, `=content`, `name=content`, or a file (`@f`, `name@f`).
    fn data_urlencode(&mut self, value: &str) {
        let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
        match value.split_once(['=', '@']) {
            Some((name, _)) if value.as_bytes()[name.len()] == b'@' => self.file(&value[name.len() + 1..]),
            Some(("", content)) => self.data.push(Data::Encoded(enc(content))),
            Some((name, content)) => self.data.push(Data::Encoded(format!("{name}={}", enc(content)))),
            None => self.data.push(Data::Encoded(enc(value))),
        }
    }

    /// `-F name=value`, `-F name=@file` (a file), `-F name=<file` (a file's text). `--form-string`
    /// takes the value literally.
    fn form_field(&mut self, raw: &str, special: bool) {
        let Some((name, value)) = raw.split_once('=') else {
            self.warnings.push(format!("Skipped the form field `{raw}`, which has no `=`."));
            return;
        };
        if special && let Some(path) = value.strip_prefix('@') {
            let path = path.split(';').next().unwrap_or(path);
            self.warnings.push(format!("Choose the file for the form field `{name}` (`{path}`) in the Body tab."));
            self.form.push((name.to_owned(), String::new(), true));
        } else if special && let Some(path) = value.strip_prefix('<') {
            self.warnings.push(format!("The form field `{name}` was read from `{path}`; fill it in the Body tab."));
            self.form.push((name.to_owned(), String::new(), false));
        } else {
            self.form.push((name.to_owned(), value.to_owned(), false));
        }
    }

    fn finish(mut self) -> Result<CurlParseResult, String> {
        let raw_url = self.url.take().ok_or("the curl command has no URL")?;
        let with_scheme = if raw_url.contains("://") { raw_url.clone() } else { format!("http://{raw_url}") };
        let (mut url, mut query) = match Url::parse(&with_scheme) {
            Ok(mut parsed) => {
                let query: Vec<KeyValue> = parsed.query_pairs().map(|(k, v)| kv(&k, &v)).collect();
                parsed.set_query(None);
                parsed.set_fragment(None);
                (parsed.to_string(), query)
            }
            Err(e) => {
                self.warnings.push(format!("The URL didn't parse ({e}); it was kept as written."));
                (raw_url.clone(), Vec::new())
            }
        };
        // `Url` adds a `/` to a bare origin; keep the URL as the user wrote it.
        if !with_scheme.trim_end_matches(['?', '#']).ends_with('/')
            && url.ends_with('/')
            && url.matches('/').count() == 3
        {
            url.pop();
        }
        for extra in std::mem::take(&mut self.url_query) {
            query.extend(url::form_urlencoded::parse(extra.as_bytes()).map(|(k, v)| kv(&k, &v)));
        }

        let data: Vec<String> = std::mem::take(&mut self.data)
            .into_iter()
            .map(|d| match d {
                Data::Text(s) | Data::Encoded(s) => s,
            })
            .collect();
        let has_body = !data.is_empty() || !self.json.is_empty() || !self.form.is_empty();
        let method = self.method(has_body);

        let mut body = Body::None;
        if self.get && !data.is_empty() {
            // `-G` sends the data as the query string instead.
            query.extend(url::form_urlencoded::parse(data.join("&").as_bytes()).map(|(k, v)| kv(&k, &v)));
        } else if !self.form.is_empty() {
            self.remove_header("content-type", |ct| ct.starts_with("multipart/form-data"));
            let fields = self
                .form
                .iter()
                .map(|(key, value, file)| FormField {
                    key: key.clone(),
                    kind: if *file { FieldKind::File } else { FieldKind::Text },
                    value: value.clone(),
                    file: None,
                    enabled: true,
                })
                .collect();
            body = Body::Multipart { fields };
        } else if !self.json.is_empty() {
            body = Body::Json { content: self.json.concat() };
            self.remove_header("content-type", |ct| ct == "application/json");
        } else if !data.is_empty() {
            body = self.data_body(data.join("&"));
        }

        let auth = self.auth();
        let name = endpoint_name(&url);
        let headers = self.headers.iter().map(|(k, v)| kv(k, v)).collect();
        let endpoint = Endpoint {
            id: Uuid::new_v4(),
            name,
            group: None,
            method,
            url,
            headers,
            query,
            body,
            auth,
            expect: None,
            extract: Vec::new(),
        };
        self.warnings.dedup();
        Ok(CurlParseResult { endpoint, warnings: self.warnings })
    }

    fn method(&mut self, has_body: bool) -> HttpMethod {
        let implied = if self.head {
            HttpMethod::Head
        } else if has_body && !self.get {
            HttpMethod::Post
        } else {
            HttpMethod::Get
        };
        let Some(raw) = self.method.take() else { return implied };
        match raw.to_ascii_uppercase().as_str() {
            "GET" => HttpMethod::Get,
            "POST" => HttpMethod::Post,
            "PUT" => HttpMethod::Put,
            "PATCH" => HttpMethod::Patch,
            "DELETE" => HttpMethod::Delete,
            "HEAD" => HttpMethod::Head,
            "OPTIONS" => HttpMethod::Options,
            _ => {
                self.warnings.push(format!("The method {raw} isn't supported; used {} instead.", method_name(implied)));
                implied
            }
        }
    }

    /// The body for `-d` data: JSON, a form, or raw text with its content type.
    fn data_body(&mut self, data: String) -> Body {
        let content_type = self.header_value("content-type");
        let mime = content_type.as_deref().map(|ct| ct.split(';').next().unwrap_or("").trim().to_ascii_lowercase());
        let looks_json = matches!(data.trim_start().chars().next(), Some('{' | '['));
        match mime.as_deref() {
            Some("application/json") => {
                self.remove_header("content-type", |_| true);
                Body::Json { content: data }
            }
            None if looks_json => Body::Json { content: data },
            None | Some("application/x-www-form-urlencoded") if is_form(&data) => {
                self.remove_header("content-type", |_| true);
                let fields = url::form_urlencoded::parse(data.as_bytes()).map(|(k, v)| kv(&k, &v)).collect();
                Body::Form { fields }
            }
            _ => {
                self.remove_header("content-type", |_| true);
                Body::Raw {
                    content_type: content_type.unwrap_or_else(|| "application/x-www-form-urlencoded".into()),
                    content: data,
                }
            }
        }
    }

    /// Bearer, Basic (`-u` or an `Authorization` header) or none. The header becomes the request's
    /// auth so it shows in the Auth tab; credentials get a warning, since they're stored as text.
    fn auth(&mut self) -> Auth {
        let mut auth = Auth::None;
        if let Some(token) = self.bearer.take() {
            auth = Auth::Bearer { token };
        } else if let Some(user) = self.user.take() {
            let (username, password) = user.split_once(':').unwrap_or((&user, ""));
            auth = Auth::Basic { username: username.to_owned(), password: password.to_owned() };
        } else if let Some(value) = self.header_value("authorization") {
            let (scheme, rest) = value.split_once(' ').unwrap_or((&value, ""));
            let rest = rest.trim();
            if scheme.eq_ignore_ascii_case("bearer") && !rest.is_empty() {
                auth = Auth::Bearer { token: rest.to_owned() };
            } else if scheme.eq_ignore_ascii_case("basic") {
                let decoded =
                    base64::engine::general_purpose::STANDARD.decode(rest).ok().and_then(|b| String::from_utf8(b).ok());
                if let Some((username, password)) = decoded.as_deref().and_then(|d| d.split_once(':')) {
                    auth = Auth::Basic { username: username.to_owned(), password: password.to_owned() };
                }
            }
            if !matches!(auth, Auth::None) {
                self.remove_header("authorization", |_| true);
            }
        }
        let secret_headers = ["authorization", "cookie", "x-api-key", "api-key", "proxy-authorization"];
        let credentials = !matches!(auth, Auth::None)
            || self.headers.iter().any(|(k, _)| secret_headers.contains(&k.to_ascii_lowercase().as_str()));
        if credentials {
            self.warnings.push(
                "This request carries credentials, which are saved in the collection as plain text. \
                 Consider moving them into an environment secret and using {{name}} instead."
                    .into(),
            );
        }
        // Lengths change with every edit; the engine sets it.
        self.remove_header("content-length", |_| true);
        auth
    }

    fn header_value(&self, name: &str) -> Option<String> {
        self.headers.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone())
    }

    /// Removes headers called `name` whose lowercased value passes `when`.
    fn remove_header(&mut self, name: &str, when: impl Fn(&str) -> bool) {
        self.headers.retain(|(k, v)| !(k.eq_ignore_ascii_case(name) && when(&v.to_ascii_lowercase())));
    }
}

fn method_name(m: HttpMethod) -> &'static str {
    match m {
        HttpMethod::Get => "GET",
        HttpMethod::Post => "POST",
        HttpMethod::Put => "PUT",
        HttpMethod::Patch => "PATCH",
        HttpMethod::Delete => "DELETE",
        HttpMethod::Head => "HEAD",
        HttpMethod::Options => "OPTIONS",
    }
}

fn kv(key: &str, value: &str) -> KeyValue {
    KeyValue { key: key.to_owned(), value: value.to_owned(), enabled: true }
}

/// `a=1&b=2`: every part is a `key=value` pair (or a bare key) with no spaces or newlines.
fn is_form(data: &str) -> bool {
    !data.is_empty()
        && data.contains('=')
        && data.split('&').all(|part| !part.is_empty() && !part.contains(char::is_whitespace))
}

/// The URL's path (`/users/42`), or its host for the root.
fn endpoint_name(url: &str) -> String {
    match Url::parse(url) {
        Ok(u) if u.path() != "/" && !u.path().is_empty() => u.path().to_owned(),
        Ok(u) => u.host_str().unwrap_or(url).to_owned(),
        Err(_) => url.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(cmd: &str) -> CurlParseResult {
        parse_one(cmd).unwrap_or_else(|e| panic!("{cmd}: {e}"))
    }

    fn pairs(kvs: &[KeyValue]) -> Vec<(&str, &str)> {
        kvs.iter().map(|k| (k.key.as_str(), k.value.as_str())).collect()
    }

    fn header<'a>(e: &'a Endpoint, name: &str) -> Option<&'a str> {
        e.headers.iter().find(|h| h.key.eq_ignore_ascii_case(name)).map(|h| h.value.as_str())
    }

    #[test]
    fn a_simple_get_with_query_and_headers() {
        let r = one("curl 'https://api.example.com/users?page=2&q=a%20b' -H 'Accept: application/json' -sSL");
        let e = &r.endpoint;
        assert_eq!(
            (e.method, e.url.as_str(), e.name.as_str()),
            (HttpMethod::Get, "https://api.example.com/users", "/users")
        );
        assert_eq!(pairs(&e.query), [("page", "2"), ("q", "a b")], "query split out and decoded");
        assert_eq!(header(e, "accept"), Some("application/json"));
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    }

    #[test]
    fn json_post_with_bearer_from_a_continued_bash_command() {
        let r = one("curl -X POST https://api.example.com/orders \\\n  -H \"Content-Type: application/json\" \\\n  \
             -H 'Authorization: Bearer abc.def' \\\n  --data-raw '{\"id\": 1, \"note\": \"it'\\''s\"}'");
        let e = &r.endpoint;
        assert_eq!(e.method, HttpMethod::Post);
        assert!(matches!(&e.body, Body::Json { content } if content == r#"{"id": 1, "note": "it's"}"#), "{:?}", e.body);
        assert!(matches!(&e.auth, Auth::Bearer { token } if token == "abc.def"));
        assert_eq!(header(e, "content-type"), None, "implied by the JSON body");
        assert_eq!(header(e, "authorization"), None, "moved to Auth");
        assert!(r.warnings.iter().any(|w| w.contains("plain text")), "{:?}", r.warnings);
    }

    #[test]
    fn chrome_cmd_format() {
        let cmd = "curl ^\"https://example.com/api/items?x=1^\" ^\n  -H ^\"accept: */*^\" ^\n  \
                   --data-raw ^\"^{^\\^\"a^\\^\":1^}^\" ^\n  --compressed";
        let e = one(cmd).endpoint;
        assert_eq!((e.method, e.url.as_str()), (HttpMethod::Post, "https://example.com/api/items"));
        assert_eq!(header(&e, "accept"), Some("*/*"));
        assert!(matches!(&e.body, Body::Json { content } if content == r#"{"a":1}"#), "{:?}", e.body);
    }

    #[test]
    fn ansi_c_quoted_body() {
        let e = one("curl https://x.io/a --data-raw $'{\"t\":\"line\\nnext \\u00e9\"}'").endpoint;
        assert!(matches!(&e.body, Body::Json { content } if content == "{\"t\":\"line\nnext é\"}"), "{:?}", e.body);
    }

    #[test]
    fn form_data_and_get_with_data() {
        let e = one("curl https://x.io/login -d 'user=ada' -d 'pass=s%20e' --data-urlencode 'note=a&b'").endpoint;
        assert_eq!(e.method, HttpMethod::Post);
        match &e.body {
            Body::Form { fields } => {
                assert_eq!(pairs(fields), [("user", "ada"), ("pass", "s e"), ("note", "a&b")]);
            }
            other => panic!("{other:?}"),
        }
        let g = one("curl -G https://x.io/search -d q=kestrel -d n=5").endpoint;
        assert!(g.method == HttpMethod::Get && matches!(g.body, Body::None));
        assert_eq!(pairs(&g.query), [("q", "kestrel"), ("n", "5")]);
        let unquoted = one("curl https://x.io/s?a=1&b=2").endpoint;
        assert_eq!(pairs(&unquoted.query), [("a", "1"), ("b", "2")], "an unquoted & stays in the URL");
    }

    #[test]
    fn raw_bodies_keep_their_content_type() {
        let e = one("curl https://x.io/x -H 'Content-Type: text/plain' -d 'hello there'").endpoint;
        assert!(
            matches!(&e.body, Body::Raw { content_type, content } if content_type == "text/plain" && content == "hello there")
        );
        assert_eq!(header(&e, "content-type"), None);
        let plain = one("curl https://x.io/x -d 'just text'").endpoint;
        assert!(
            matches!(&plain.body, Body::Raw { content_type, .. } if content_type == "application/x-www-form-urlencoded")
        );
    }

    #[test]
    fn multipart_basic_auth_and_odd_options() {
        let r = one(
            "curl -u ada:pw -F name=Ada -F 'avatar=@me.png;type=image/png' -XPUT -o out.json -m 5 http://x.io/me --frobnicate",
        );
        let e = &r.endpoint;
        assert_eq!(e.method, HttpMethod::Put, "-X attached, and -o/-m values skipped rather than taken as the URL");
        assert_eq!(e.url, "http://x.io/me");
        assert!(matches!(&e.auth, Auth::Basic { username, password } if username == "ada" && password == "pw"));
        match &e.body {
            Body::Multipart { fields } => {
                assert_eq!((fields[0].kind, fields[1].kind), (FieldKind::Text, FieldKind::File));
                assert_eq!(fields[0].value, "Ada");
            }
            other => panic!("{other:?}"),
        }
        for expected in ["me.png", "--frobnicate", "plain text"] {
            assert!(r.warnings.iter().any(|w| w.contains(expected)), "{expected}: {:?}", r.warnings);
        }
        let basic = one("curl https://x.io -H 'Authorization: Basic YWRhOnB3'").endpoint;
        assert!(matches!(&basic.auth, Auth::Basic { username, .. } if username == "ada"));
        assert_eq!(basic.url, "https://x.io", "no trailing slash added");
    }

    #[test]
    fn errors_say_what_is_wrong() {
        assert!(parse_one("wget https://x.io").unwrap_err().contains("no curl command"));
        assert!(parse_one("curl -H 'a: b'").unwrap_err().contains("no URL"));
        assert!(parse_one("curl 'https://x.io").unwrap_err().contains("isn't closed"));
        assert!(parse_one("curl -X").unwrap_err().contains("needs a value"));
    }

    #[test]
    fn several_commands_make_a_collection_on_one_base() {
        let text = "curl 'https://api.x.io/a' ;\ncurl 'https://api.x.io/b?id=1' -X DELETE\n# a comment\n$ curl https://api.x.io/c -d '{}'";
        assert!(looks_like_curl(text));
        assert!(!looks_like_curl("openapi: 3.0.0"));
        let r = import(text, None).unwrap();
        let c = &r.collection;
        assert_eq!((c.name.as_str(), c.vars["base"].as_str(), c.endpoints.len()), ("api.x.io", "https://api.x.io", 3));
        let urls: Vec<&str> = c.endpoints.iter().map(|e| e.url.as_str()).collect();
        assert_eq!(urls, ["{{base}}/a", "{{base}}/b", "{{base}}/c"]);
        assert_eq!(c.endpoints[1].method, HttpMethod::Delete);
        assert_eq!(c.source.as_deref(), Some("curl · 3 requests"));
        let mixed = import("curl https://a.io/x\ncurl https://b.io/y", Some("Mine")).unwrap().collection;
        assert!(mixed.vars.is_empty() && mixed.endpoints[0].url == "https://a.io/x" && mixed.name == "Mine");
        let first = parse_one(text).unwrap();
        assert!(first.warnings.iter().any(|w| w.contains("first of 3")));
    }
}
