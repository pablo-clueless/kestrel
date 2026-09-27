# Kestrel — Handoff

> Status: **M2 built** (2026-09-27). Load test with both models, in-flight cap with dropped counting,
> scheduler-lag tracking, error classification (`okStatuses`), host confirmation (`/api/hosts`), no
> redirects under load, and live dropped/in-flight/lag in the UI. 68 engine tests pass, including
> coordinated omission, dropped sends, redirects, host confirmation and the rate-cap pacer. Measured live
> on a release build against the reference target:
> open 1,000 req/s on `/fast` → 999.97 req/s, p50 0.14 ms, generator lag p99 0.04 ms, 0 dropped;
> `/stall` open 200 req/s → p90 306 ms / p99 485 ms vs closed 1 user → p99 0.15 ms (max 515 ms).
>
> M2 notes:
> - The open-model scheduler runs on a blocking thread and spin-waits for the last 2 ms before each
>   send. Tokio's 1 ms timer wheel made every send ~1 ms late, and in the open model lateness is
>   latency. This costs up to one core at high rates.
> - Closed-model users are paced to `max_rps` too. Without that, 50 users against a fast target sent
>   77k req/s, well past the 1,000 req/s cap. The report says when the cap throttled a run.
> - Host confirmations are held in memory for the engine's lifetime, not persisted.
> - The file-descriptor-limit check isn't implemented. It's Unix-only and this is being built on
>   Windows.
> - Use `cargo run --release -p engine` for load tests; a debug engine is a slower generator.
>
> Earlier: **M1 built** (2026-09-27). On top of M0: workspace + secrets files, templating, `/render`,
> `/send`, the Latency probe (all/ok/TTFB percentiles, histogram, cold request, status + error classes,
> redacted samples), DNS pinning, the redirect policy, and the reference target (`--example target`).
> 60 engine tests pass, clippy is clean, and `next build` passes. Checked live: httpbin ×20 with the echoed
> `Authorization` redacted, `/sleep?ms=20` → p50 20.7 ms, `/fast` → p50 0.2 ms, `/flaky` → ~10% `http`
> errors. **Not yet checked by eye in a browser:** the M0 loop and the M1 UI (request editor, environments,
> Send, Run → report).
>
> M1 notes: redirects in latency/send are followed only to the same host or loopback; the
> confirmed-host store (needed for other hosts) arrives with load tests in M2. The body editor is a
> textarea; Monaco is deferred. Secret values shorter than 3 characters aren't scrubbed from bodies.
>
> **Deviations from the plan below:** the Next.js app lives in `kestrel/` (not `web/`), so generated
> types are in `kestrel/src/types/engine/`. The single `.env` is at the repo root; `kestrel/next.config.ts`
> loads it with `process.loadEnvFile` and maps `KESTREL_TOKEN` → `NEXT_PUBLIC_KESTREL_TOKEN`, so the
> token is set once. Guard rejections are 403 (415 for a wrong content type).
> Last updated: 2026-09-27 (rev 2: design review fixes folded in, see [Changes in rev 2](#changes-in-rev-2))

## What we're building

A single-screen app for performance-testing HTTP endpoints. You load endpoints
(from an API spec or by hand), pick one, pick a test, hit Run, and watch the
results chart live.

Tests it runs: latency, load, stress/breakpoint, spike, soak, empirical Big-O
(how latency grows with input size), plus cheap extras like contract validation and
rate-limit discovery. Details in [Test catalogue](#test-catalogue).

**Non-goals (for now):** distributed load generation across machines, browser/UI
testing, gRPC/WebSocket/GraphQL-subscription testing, CI mode, user accounts,
multi-step scenarios (login → use token → …).

## Changes in rev 2

Read this first if you saw rev 1. Everything below is already reflected in the rest of the doc.

- **Engine API is locked down.** Binding to 127.0.0.1 alone does not stop a web page from driving the
  engine. Every request now needs a session token, a valid `Host`, an allowlisted `Origin` (when
  present) and a JSON content type. Caps can only be raised from local config / CLI, never via the API.
  See [Safety rails](#safety-rails).
- **All API routes live under `/api`** from M0, in dev and in single-binary mode.
- **Target hosts are resolved once and pinned** for the run. Redirects are not followed during
  load-type tests.
- **Secrets move to a separate gitignored file**, and redaction covers sampled request/response
  headers and bodies, not just environment variables.
- **Open-model load has an in-flight cap.** Sends that can't be issued are counted as dropped and
  surfaced, never silently skipped.
- **Failed and timed-out requests are in the latency data.** Reports carry both "all requests" and
  "successful only" histograms, labelled.
- **Error classification is explicit** and configurable per test.
- **SSE is replayable.** Events have ids, the engine keeps a bounded history per run, and the client
  always fetches the final report over REST.
- **Big-O fitting fixed:** dropped 2ⁿ, constant is its own model, no baseline subtraction before
  fitting, randomised sweep order, fresh values every sample, payload-size baseline, per-sweep time
  budget.
- **Decided: one endpoint per run in v1.** Weighted multi-endpoint mixes move to v1.x. (Default
  chosen during review; change it if needed, but decide before M2 because it shapes the `Runner` trait.)
- **Soak** needs an explicit duration override and supports static credentials only.
- Small M0 gotchas noted: `EventSource` can't send headers, `http::Method` / `Uuid` need work for `ts-rs`.

## Shape: Next.js UI + Rust engine

Two parts, one repo:

- **`web/`: Next.js frontend.** The one screen. Handles editing, config, and live charts.
  It never sends test traffic itself.
- **`engine/`: Rust service (axum).** Handles spec import, templating, the load generator, stats and
  Big-O fitting. It sends all test traffic.

Why not do the requests from the browser or from Next.js:
- **Browser:** CORS blocks requests to most APIs. Browsers also cap about 6 connections per host,
  and `fetch` timing is too coarse and noisy for percentiles.
- **Node (Next.js route handlers):** it can do it, but a single-threaded event loop becomes the
  bottleneck (and skews latency) well before a tokio runtime does. Coordinated-omission-correct
  open-model scheduling is also easier to get right in Rust.

So the browser talks only to the engine on `127.0.0.1`, and the engine talks to the targets.

```
 Browser (Next.js UI) ──REST /api──▶ engine :7070 ──HTTP load──▶ target APIs
        ▲                              │
        └──────── SSE run events ──────┘
```

### Frontend stack (`web/`)

| Concern | Choice | Notes |
|---|---|---|
| Framework | Next.js (App Router), TypeScript | Build with `output: 'export'` so it can be served as static files by the engine later (single binary). This means no server actions and no route handlers: all data comes from the engine |
| Styling / components | Tailwind + shadcn/ui | Resizable panels (`ResizablePanelGroup`), tabs, dialogs, command palette for endpoint search |
| Client state | Zustand | Selected endpoint, draft request edits, active run's live buffer |
| Server state | TanStack Query | Workspace, endpoints, environments, saved reports (engine REST) |
| Charts | Recharts | Live latency/RPS time series, histograms, log-log Big-O plot (`scale="log"`). Engine pre-buckets data so charts stay at ≤ 4 updates/s |
| Code editor | Monaco (`@monaco-editor/react`) | Body editor with JSON highlighting + `{{template}}` hints |
| Types | Generated from Rust via `ts-rs` into `web/src/types/engine/` | One source of truth for `Endpoint`, `RunEvent`, `RunReport` |

### Engine stack (`engine/`)

| Concern | Choice | Why |
|---|---|---|
| HTTP API | `axum` + `tower-http` | REST + SSE for run events. CORS allowlist is `http://localhost:3000` and `http://127.0.0.1:3000` in dev only |
| Async runtime | `tokio` (multi-thread) | Thousands of concurrent in-flight requests |
| HTTP client | `reqwest` (rustls), redirects disabled for load-type tests. Drop to `hyper` + a custom connector for phase timings | See [Latency](#1-latency-probe) |
| DNS pinning | `reqwest::ClientBuilder::resolve` with the IP resolved at run start | See [Safety rails](#safety-rails) |
| Latency stats | `hdrhistogram` | Accurate percentiles at high sample counts, cheap to merge. Max trackable value must exceed the largest configured timeout |
| OpenAPI 3.0 / 3.1 | `openapiv3` / `oas3` | `openapiv3` doesn't cover 3.1 |
| YAML | a maintained fork (`serde_yaml_ng` or `serde_norway`) | `serde_yaml` is archived |
| Schema validation | `jsonschema` | Contract checks against response schemas |
| Cancellation | `tokio-util::CancellationToken` | The Stop button must kill a run immediately |
| TS type export | `ts-rs` (with `uuid-impl` feature) | Use our own `HttpMethod` enum; `http::Method` doesn't derive `TS` |
| Errors | `anyhow` / `thiserror` | |

Big-O fitting is a small least-squares routine. We'll write it ourselves, with no dependency.

## The one screen

```
┌───────────────────────────────────────────────────────────────────────────────┐
│ [Import spec ▾] [+ Endpoint]  Env: [local ▾]  Base URL: http://localhost:8080 │
├──────────────────────┬────────────────────────────────────────────────────────┤
│ ENDPOINTS            │ REQUEST                                                │
│ ▸ users              │  POST  {{base}}/users/search                           │
│   GET  /users        │  Headers | Query | Body | Auth                         │
│ ● POST /users/search │  { "ids": {{n:int_array}} }                            │
│   GET  /users/{id}   │                                                        │
│ ▸ orders             ├────────────────────────────────────────────────────────┤
│   ...                │ TEST  [Latency][Load][Stress][Spike][Soak][Big-O][...] │
│                      │  per-test config (concurrency, rate, duration, n-range)│
│ [filter...]          │                                    [▶ Run] [■ Stop]    │
│                      ├────────────────────────────────────────────────────────┤
│                      │ RESULTS                                                │
│                      │  live chart (latency over time / RPS / curve fit)      │
│                      │  p50 12ms  p90 30ms  p99 81ms  err 0.2%  rps 1,204     │
│                      │  status: 200×11,980  429×24   dropped 0   lag ok       │
│                      │                              [Export JSON] [Export CSV]│
└──────────────────────┴────────────────────────────────────────────────────────┘
```

The left panel is the endpoint list, grouped by tag or path prefix. The right side holds the request
editor, the test picker and its config, and the results. Sections collapse and resize, and
there are no extra routes: the whole app is `app/page.tsx`.

## Inputs

Everything gets normalised into one internal model (below). Importers live in the engine
as pure functions `bytes -> Result<Vec<Endpoint>>`, so they're easy to unit-test.
The UI uploads the file (or pasted text) to `POST /api/import` and gets endpoints back.

| Source | Priority | Notes |
|---|---|---|
| Manual entry | v1 | Method, URL, headers, query, body |
| OpenAPI 3.0 / 3.1 (JSON or YAML) | v1 | Use `servers[0]` as base URL. Build example bodies from `example`/`examples`, or synthesize them from the schema if there are none |
| Swagger 2.0 | v1.x | Convert to the same model |
| Paste a `curl` command | v1.x | Very common way people share requests |
| Postman collection v2.1 | v1.x | Folders become groups; collection variables become the environment |
| HAR file | later | Replays real browser traffic |
| Free-form docs (Markdown/HTML) | later | Would need an LLM extraction step. Out of scope for v1 |

### Internal model (sketch, Rust, exported to TS)

```rust
#[derive(Serialize, Deserialize, TS)]
struct Endpoint {
    id: Uuid,
    group: Option<String>,        // tag / folder
    method: HttpMethod,           // own enum, not http::Method
    url: Template,                // "{{base}}/users/{{id}}"
    headers: Vec<(String, Template)>,
    query: Vec<(String, Template)>,
    body: Option<Body>,           // Json(Template) | Form | Raw { content_type, Template }
    auth: Auth,                   // None | Bearer | Basic | ApiKey { in, name }
    expect: Option<Expectation>,  // status codes + response schema, from spec
}

struct Environment { name: String, vars: BTreeMap<String, String> }  // base, ...
struct Secrets { by_env: BTreeMap<String, BTreeMap<String, String>> } // token, ...
```

**Templating:** `{{var}}` pulls from the environment, then from secrets. Generators produce a fresh
value per request: `{{uuid}}`, `{{int:1..1000}}`, `{{seq}}`. For Big-O there are also
size-parameterised generators driven by `n`:
`{{n}}`, `{{n:int_array}}`, `{{n:string}}`, `{{n:object_array:<schema>}}`.
Rendering happens in the engine. The UI calls `POST /api/render` for a preview.

Templating rules:
- Templates are parsed once when a run starts, not per request.
- `{{seq}}` is a per-run `AtomicU64`, so it stays unique under concurrency.
- `n` generators produce **fresh random contents** on every request, not just the right size,
  so server-side caching can't flatten Big-O results.
- In the closed model, the request is rendered *before* the send timestamp is taken. In the open
  model, latency is measured from the scheduled time, so render cost counts (keep it cheap).

### Workspace files

- `kestrel.json`: endpoints, environments and saved test configs. Safe to commit.
- `kestrel.secrets.json`: secret values per environment. The engine creates it next to
  `kestrel.json` and adds it to `.gitignore` if a `.gitignore` exists there. Never exported.
- Both are written atomically (write to a temp file, `fsync`, rename).
- `PUT /api/workspace` replaces the whole document. Two tabs editing at once will overwrite each
  other; accepted for v1.

Results export separately.

## Engine API

All routes are under `/api`, in dev and in single-binary mode, so the typed client has one base path.

| Method | Path | Purpose |
|---|---|---|
| GET/PUT | `/api/workspace` | Load / save endpoints + environments (secrets are write-only: GET returns masked values) |
| POST | `/api/import` | Spec file or pasted text → `Endpoint[]` |
| POST | `/api/render` | Preview a rendered request (templating, secrets masked) |
| POST | `/api/send` | Fire one request, return the full response + timings (the "try it" button) |
| POST | `/api/hosts/confirm` | Record "I own or am authorised to test this host" (see Safety rails) |
| POST | `/api/runs` | Start a test → `{ runId }` |
| GET | `/api/runs/:id/events` | **SSE** stream of `RunEvent` (bucketed every 250 ms, then `RunFinished`) |
| DELETE | `/api/runs/:id` | Stop |
| GET | `/api/runs/:id/report` | Final report (JSON; `?format=csv`) |

We use SSE rather than WebSocket because the stream only goes one way, `EventSource`
reconnects automatically, and it's simpler on both ends. The browser connects to the engine directly
(CORS in dev, same origin in single-binary mode), not through Next.js rewrites, because proxied SSE
can get buffered.

### SSE semantics

- Every event has a monotonically increasing `id:` per run.
- The engine keeps a bounded history of events per run (enough for the whole of a default-length run,
  downsampled beyond that). A new subscriber gets the history replayed, then live events. Honour
  `Last-Event-ID` on reconnect and resume after it.
- This means it doesn't matter whether the client subscribes before or after the run produces its
  first bucket.
- A receiver that falls behind gets `RecvError::Lagged` from `broadcast`. Handle it by sending the
  client a `Resync` event and replaying from history, instead of ending the stream.
- The last event is `RunFinished { status: completed | cancelled | failed }`. The client then always
  fetches `GET /api/runs/:id/report`. The report is never delivered only over SSE.
- Completed runs stay in the registry for 30 minutes after finishing (report persisted), then are evicted.

## Test catalogue

Every test produces a `RunReport`. It holds:
- the config and a `status` (`completed`, `cancelled`, `failed`)
- two histograms: **all requests** (failures and timeouts included) and **successful only**
- status counts, error counts by class, dropped/late send counts, generator lag stats
- a few error samples (redacted, see Safety rails)
- time-series buckets

The UI renders it and export serialises it. Charts label which histogram they show; the headline
percentiles use **all requests**.

**Timeouts** are recorded in the "all requests" histogram at the timeout value and counted as
errors. The histogram's max trackable value is set above the largest configured timeout.

**Error classification** is part of every test's config, with these defaults:
- Transport errors (DNS, connect, TLS, reset, timeout): always errors.
- 5xx: error.
- 4xx: error, except status codes the endpoint's spec declares for that operation.
- 429: counted separately as `rate_limited`, and as an error unless the test is Rate-limit discovery.

A cancelled run still produces a report with the data collected up to the Stop.

**v1 runs target one endpoint.** Weighted multi-endpoint mixes are v1.x.

### v1

#### 1. Latency probe
Sends W warm-up requests (default 10), then N sequential requests (default 100).
Reports min/mean/p50/p90/p99/max, stddev, and a histogram.

- **Phase breakdown** (DNS → TCP connect → TLS → TTFB → body download) is the most useful
  part. `reqwest` doesn't expose the connection phases, so this needs a custom `hyper` connector
  that timestamps each stage. Ship total time + TTFB first (TTFB is when `send()` resolves with
  headers), then the phases.
- **Keep-alive on/off toggle:** shows how much connection setup costs.
- **Cold vs warm:** the first request is reported separately from the rest.

#### 2. Load test
There are two modes, because they answer different questions:

- **Closed model (fixed concurrency):** C virtual users. Each one sends, waits for the response,
  then sends again. Answers "How does it behave with 50 clients hammering it?" Note in the UI that
  throughput falls when the server slows, so this model understates how bad a stall is for real
  arrival traffic.
- **Open model (fixed arrival rate):** requests are scheduled at R req/s, whether or not
  earlier ones have finished. Answers "Can it handle 500 rps?"

The open model must measure latency from the **scheduled** send time, not the actual
send time. Otherwise the results suffer from coordinated omission: a stalled server hides its own
slowness. This is the single most common bug in home-grown load tools, so test for it
explicitly.

**In-flight cap (open model):** in-flight requests are capped (default 10,000, configurable within
local limits). When the cap is hit, a scheduled send that can't be issued is **counted as dropped**
and shown in the live stats and the report. It is never silently skipped, because that just moves
the coordinated omission somewhere else.

**Connection hygiene:**
- With keep-alive off at high rps, the client can run out of ephemeral ports (TIME_WAIT). Show a
  warning in the config panel, and classify `EADDRNOTAVAIL`-style failures as `client_resource`
  errors, not target errors.
- The engine checks its file descriptor limit at startup and warns if it's below what the configured
  in-flight cap needs.

Config: duration, concurrency or rate, optional ramp-up, timeout, error classification. Shown live:
rps, p50/p99 over time, error rate, status distribution, dropped count, generator lag.

#### 3. Big-O (empirical complexity)
Answers "does this endpoint get slower linearly, quadratically, ... as input grows?"

1. The user marks the size parameter with an `n` generator: body array length, a `limit`
   query param, string length, etc.
2. Pick n values over a geometric series, e.g. 1, 2, 4 … 4096 (configurable).
3. **Sample in randomised, interleaved order** across n values (e.g. shuffled rounds), not one n at a
   time from small to large. Otherwise warm-up, GC, JIT and cache effects line up with n.
4. Warm up once at the start, then take K samples per n (default 30) with fresh random contents
   every time, and keep the **median**, which is robust to network jitter.
5. **Payload baseline:** record the request and response bytes at each n. Optionally run the same
   bodies against an echo endpoint (the target's `/echo`, or one the user picks) and show that curve
   alongside, so the user can see how much of the growth is just transfer and parsing.
6. Fit candidate models by least squares:
   - constant: `t(n) = a` (one parameter, fitted separately)
   - `t(n) = a + b·f(n)` for f in {log n, n, n log n, n², n³}
   - 2ⁿ is not a candidate (it overflows and means nothing at these n).
   The intercept `a` absorbs the network floor, so **don't subtract a baseline before fitting**.
   Constrain `b ≥ 0`.
7. Rank by R². As a second signal, compute the slope of log(t − â) against log n over the upper half
   of the range (≈0 constant, ≈1 linear, ≈2 quadratic) and show it. Use it to break close calls
   between n and n log n, and flag "n vs n log n: can't separate at this range" when R² values are
   within a small margin.
8. Show all fits on a log-log plot with the best one highlighted.
9. Report a **confidence** label. If the spread of samples at each n (IQR) is bigger than the growth
   across n, report "inconclusive" instead of guessing.
10. **Time budget:** the sweep has a total time limit (default 5 min) and a per-request limit. If a
    median at some n exceeds the per-request limit, stop increasing n and report the range actually
    covered.

Caveats to show in the UI:
- It measures server + network + serialisation time, not the algorithm in isolation.
- Caching can make an endpoint look O(1).
- The results only hold for the range of n that was tested.

#### 4. Contract check
Runs automatically alongside any test when the endpoint came from a spec. It checks that the status code
is one the spec declares and that the response body validates against the declared schema. It counts
violations and keeps a few samples (redacted). It's cheap, and it catches endpoints that are "fast
because they're returning an error". Under high load, validate a sample of responses (default: up to
50/s) rather than all of them, so validation doesn't slow the generator.

### v1.x

| Test | What it does | Output |
|---|---|---|
| **Stress / breakpoint** | Steps the rate up (e.g. +10% every 30 s) until error rate > X% or p99 > Y ms | The rps where it broke, and the curve leading up to it |
| **Spike** | Runs a baseline rate, then a sudden burst (e.g. 10×) for a short window, then back to baseline | Latency during the spike, and how long recovery takes |
| **Soak** | Moderate constant load for a long time (30 min to hours) | Latency and error drift over time, which hints at leaks or pool exhaustion |
| **Payload scaling** | Like Big-O, but varies request/response *bytes* | Throughput (MB/s) and latency vs size |
| **Rate-limit discovery** | Ramps up until 429s appear | The threshold, plus any `Retry-After` / `X-RateLimit-*` headers seen |
| **Concurrency correctness** | Fires the same mutating request N times in parallel | Status spread. Flags non-idempotent behaviour such as duplicate creates or 500s |
| **Timeout behaviour** | Uses tight client timeouts | Whether the server fails fast or hangs |
| **Multi-endpoint mix** | Weighted mix of endpoints in one load run | Per-endpoint and combined stats |

**Soak specifics:**
- Needs the duration cap raised in local config (see Safety rails); the UI says so rather than failing
  at start.
- Static credentials only. A token that expires mid-soak will show up as a wall of 401s; the UI
  warns about this before starting. Token refresh is open question 3.
- The report's time series is downsampled (e.g. to 1-minute buckets beyond the first 10 minutes), not
  just the UI's ring buffer.

## Repo layout

```
Cargo.toml              // workspace: members = ["engine"]
engine/
  Cargo.toml
  src/
    main.rs             // axum server, binds 127.0.0.1:7070
    api/                // routes: workspace, import, render, send, hosts, runs (+ SSE)
      guard.rs          // token, Host, Origin, content-type checks (tower layer)
    model/              // Endpoint, Environment, Secrets, Template, Workspace (serde + ts-rs)
    import/             // openapi.rs, swagger2.rs, curl.rs, postman.rs  (pure, testable)
    template/           // {{var}} + generator rendering
    engine/
      mod.rs            // Runner trait, RunRegistry (id → cancel token + broadcast + history)
      client.rs         // HTTP client wrapper, per-request timing, DNS pinning, redirect policy
      targets.rs        // host resolution, localhost check, confirmed-host store
      latency.rs  load.rs  complexity.rs  stress.rs ...
    stats/
      histogram.rs      // hdrhistogram wrapper (all + success), time buckets
      fit.rs            // Big-O least-squares fitting
    redact.rs           // header/body redaction for samples and exports
    report.rs           // RunReport + JSON/CSV export
  examples/target.rs    // reference test server (see below)
web/
  app/page.tsx          // the one screen
  components/           // endpoint-list, request-editor, test-config, results/*
  lib/engine.ts         // typed REST client + useRunEvents(runId) SSE hook
  stores/               // zustand: selection, drafts, live run buffer
  src/types/engine/     // ts-rs generated — do not edit
```

The current root `Cargo.toml` + `src/main.rs` stub moves into `engine/` in M0.

**Run flow:** `POST /api/runs` validates the config against the caps and the confirmed hosts,
resolves and pins the target IP, then spawns a tokio task and registers
`{ cancel: CancellationToken, tx: broadcast::Sender<RunEvent>, history: bounded buffer }`. The SSE
handler replays history and then subscribes. The engine aggregates results into 250 ms buckets, so
the browser gets at most 4 small messages per second, however many requests per second are being
sent. The UI appends each bucket to a capped ring buffer in Zustand, and Recharts renders from that.

**Load generator honesty:** the client machine itself can become the bottleneck. The engine tracks its
own scheduling lag and warns if it can't keep up with the requested rate. When the target is on the
same machine (as with the reference target), the two compete for CPU; the UI notes this when the
pinned target IP is loopback.

## Dev workflow

```bash
cp .env.example .env                        # set KESTREL_TOKEN (the UI picks it up via next.config.ts)
cargo run -p engine                         # :7070
cargo run -p engine --example target        # :8080 reference target (M1)
cd kestrel && pnpm dev                      # :3000 (restart after changing .env)
cargo test -p engine export_bindings        # regenerate TS types (ts-rs)
```

`.env` is gitignored; `.env.example` is committed with a placeholder.

Later, for a single binary: `next build` (static export) → embed `web/out` in the engine
(`rust-embed`), then serve it at `/` with the API under `/api`. In that mode the engine generates a
random token at startup and injects it into the served `index.html` as
`<meta name="kestrel-token" content="…">`; the UI reads it from there.

## Safety rails

This is a load generator, so it's easy to point it at something you shouldn't — and easy for
something else to point it for you.

### The engine API

Binding to **127.0.0.1 only** stops other machines from reaching the engine, but not web pages in
your own browser: any site can send requests to `127.0.0.1:7070`, and DNS rebinding can let it read
the responses too. So every `/api` request goes through a guard layer:

- **Session token.** Required as `X-Kestrel-Token` on every request. `EventSource` can't set
  headers, so the SSE endpoint alone accepts it as `?token=`. The token comes from `KESTREL_TOKEN`,
  or is randomly generated at startup if unset. Compare in constant time.
- **Host check.** Reject any request whose `Host` isn't `127.0.0.1:7070` or `localhost:7070` (or the
  configured port). This blocks DNS rebinding.
- **Origin check.** If an `Origin` header is present, it must be on the allowlist (the dev UI
  origins, or the engine's own origin in single-binary mode).
- **Content type.** Requests with a body must be `application/json` (or `multipart/form-data` for
  `/api/import`), which forces a CORS preflight from other origins.

### Targets

- **Localhost** means the resolved IP is in 127.0.0.0/8 or is `::1`, not the hostname.
- Target hosts are **resolved once when a run starts and pinned** for the whole run, so the
  resolution that was checked is the one that gets load.
- Before a load, stress, spike or soak test against a non-localhost target, the UI asks for a
  one-time confirmation per host ("I own or am authorised to test this"), recorded via
  `POST /api/hosts/confirm`. The engine refuses runs against unconfirmed hosts. (With the guard layer
  in place, only the real UI can make this call.)
- **Redirects are not followed** during load, stress, spike and soak tests; 3xx responses are
  recorded as-is. Latency probe, `/send` and Big-O may follow up to 5 redirects, but only to hosts
  that are localhost or confirmed.

### Caps

- Default caps are 1,000 rps, 60 s duration and 10,000 in flight.
- Caps can only be raised in `kestrel.config.toml` next to the workspace, or with CLI flags. **The API
  cannot raise them**, and the UI shows the current caps read-only.

### Stopping

- The Stop button is always visible during a run. It cancels the run's token, which drops in-flight
  futures and closes their connections. The target may still finish processing those requests.

### Secrets

- Secret values live in `kestrel.secrets.json` (gitignored), never in `kestrel.json`.
- They're masked in the UI and in `/api/render` output, and left out of exports.
- `redact.rs` scrubs sampled requests and responses before they're stored in a report: the
  `Authorization`, `Cookie`, `Set-Cookie` and `Proxy-Authorization` headers, any header named in the
  endpoint's `ApiKey` auth, and any occurrence of a secret value inside sampled bodies (services like
  httpbin echo headers back).

## Milestones

Each milestone ends with something runnable.

| # | Deliverable | Done when |
|---|---|---|
| **M0** | Cargo workspace + `engine/` axum skeleton with `/api` prefix and the guard layer. `web/` Next.js + Tailwind + shadcn with the empty panel layout. ts-rs type generation. A fake runner streams over SSE with event ids and history replay | `pnpm dev` + `cargo run` show a live Recharts line fed by fake SSE events. Stop cancels it. Reloading the page mid-run resumes the chart. A request without the token, or with a foreign `Host`, gets 403 |
| **M1** | Manual endpoint entry, environments + secrets file, templating, the `/send` try-it button, and the Latency probe | Can hit `GET https://httpbin.org/get` 100× and see percentiles + a histogram. The echoed `Authorization` header is redacted in samples |
| **M2** | Load test (closed + open model), in-flight cap, error classification, live chart, Stop | The coordinated-omission test passes (see below). Dropped sends are counted when the cap is hit against `/stall`. Reaches 1k rps against the local target with no generator-lag warnings. A redirect from a confirmed host to an unconfirmed one is not followed |
| **M3** | OpenAPI 3.0/3.1 import + contract check | The Petstore spec imports, endpoints are grouped by tag, and bad responses are flagged |
| **M4** | Big-O test + log-log plot | Correctly classifies the reference endpoints (below), including reporting `/sort` as n log n or as "n vs n log n: can't separate" rather than confidently wrong |
| **M5** | JSON/CSV export, workspace save/load, static export served by the engine with token injection | Round-trips. `cargo run -p engine` alone serves the whole app |
| **M6+** | v1.x tests, Swagger 2 / curl / Postman importers, phase timings | |

## Test target (build this in M0/M1)

A tiny axum server at `engine/examples/target.rs` with endpoints whose behaviour is **known**
in advance, so we can check the tool rather than trust it:

| Endpoint | Behaviour | Used to verify |
|---|---|---|
| `GET /fast` | Returns immediately | Latency floor, max rps |
| `GET /sleep?ms=` | Sleeps N ms | Latency accuracy |
| `GET /stall` | Server-wide pause: for 500 ms of every 2 s, every arriving request waits until the pause ends (like a GC pause) | Coordinated omission: the open-model p90/p99 must show the pause; a 1-user closed model shows it only as its max. (Originally "every 100th request sleeps 2 s", but a per-request stall doesn't block other requests, so it can't demonstrate coordinated omission.) |
| `POST /echo` | Returns the request body unchanged | Payload baseline for Big-O |
| `POST /linear` | O(n) over the input array | Big-O → O(n) |
| `POST /quadratic` | O(n²) nested loop | Big-O → O(n²), and the sweep time budget |
| `POST /sort` | Sorts the input | Big-O → O(n log n). Hard to tell apart from O(n), which makes it a good stress test for the fitter |
| `POST /cached` | O(n²) the first time it sees a body, instant afterwards | Fresh-value generators: must not classify as O(1) |
| `GET /flaky` | Returns a 500 for 10% of requests | Error-rate reporting |
| `GET /limited` | Returns 429 above 50 rps | Rate-limit discovery, `rate_limited` counting |
| `GET /redirect?to=` | 302 to the given URL | Redirect policy |
| `GET /echo-headers` | Returns request headers as JSON | Secret redaction in samples |

## Open questions

1. Should spec import live in the engine (the current plan, one source of truth, and the engine needs
   resolved schemas for contract checks anyway), or in TS with `@apidevtools/swagger-parser`
   (more mature `$ref` handling)? Revisit if `$ref` resolution in Rust becomes painful.
2. Should Big-O also support sweeping *server state*, e.g. "seed N rows first, then
   query"? That needs a setup-request hook, so probably v2.
3. Do we need auth flows beyond static tokens, such as OAuth2 client-credentials refresh? Soak is the
   first test that really needs it.
4. Should we compare runs (before/after a deploy) side by side? In v1.x or later?
5. Should we wrap it as a desktop app (Tauri) later, or is a browser tab on localhost fine?
6. One endpoint per run in v1 was chosen during review. Confirm before M2.

## Next step

Start **M0**:
1. Turn the root into a Cargo workspace, move the stub into `engine/`, and add axum, tokio,
   tokio-util, tower-http and ts-rs (`uuid-impl` feature).
2. Build the guard layer (`api/guard.rs`) first: token, Host, Origin and content-type checks, with
   tests for each rejection. Mount every route under `/api`.
3. Run `pnpm create next-app web` (App Router, TS, Tailwind), then `shadcn init`, and add the resizable
   panel layout. Add `.env.example` with `KESTREL_TOKEN` / `NEXT_PUBLIC_KESTREL_TOKEN`.
4. Wire a fake runner through `POST /api/runs` → SSE (ids + history replay + `RunFinished`) →
   Zustand → Recharts, and fetch `/api/runs/:id/report` on finish, to prove the live-update loop
   before touching real HTTP.