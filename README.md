# Kestrel

A local tool for performance-testing HTTP APIs. Point it at an endpoint, whether you typed it in or
imported it from an OpenAPI spec, and measure latency, behaviour under load, and how latency grows with
input size. Responses are checked against the spec while you do it.

It has two parts. A **Rust engine** sends every request and does the measuring, and a **Next.js UI**
drives it and charts results live. The browser never sends test traffic itself, because browsers are
too slow, too limited in connections and too imprecise for accurate numbers.

```
 Browser (Next.js UI) ──REST /api──▶ engine :7070 ──HTTP──▶ the API under test
        ▲                              │
        └──────── live results (SSE) ──┘
```

## What it measures

| Test | Answers | Notes |
|---|---|---|
| **Latency probe** | How fast is one request? | Sequential requests after a warm-up. p50/p90/p99/max for all requests, successful ones only, and time to first byte; a distribution; the first (cold) request reported separately |
| **Load test** | How does it behave under load? | **Open model:** a fixed request rate, with latency measured from the *scheduled* send time, so a stalled server can't hide its own slowness (no coordinated omission). **Closed model:** a fixed number of users. Tracks an in-flight cap, dropped sends and the load generator's own lateness |
| **Big-O** | Does latency grow like O(n), O(n log n), O(n²)…? | Sweeps an input size `n` across a geometric range in shuffled rounds with fresh random inputs, fits candidate models, and reports a verdict, a confidence level and the log-log slope. It says "can't separate" or "inconclusive" rather than guessing |
| **Spec contract** | Do responses match the spec? | Runs on Send and during runs for imported endpoints: undeclared status codes and JSON Schema mismatches, with the location of the error |

Measured against the bundled reference server on a release build: the open model holds 1,000 req/s
with p99 scheduler lateness of 0.04 ms. Big-O classifies the O(1), O(n), O(n log n) and O(n²)
reference endpoints correctly, and reports a cached O(n²) endpoint as O(n²), not O(1).

## Features

- **Collections** of endpoints, one per API. Import OpenAPI 3.0 / 3.1 or Swagger 2.0 (JSON or YAML)
  from a file, pasted text or a URL, or add endpoints by hand.
- **Environments and secrets.** `{{base}}`-style variables resolve from the active environment, then
  its secrets, then the collection. Secrets are write-only in the UI, stored in a gitignored file, and
  redacted from every response and report, including when a server echoes them back.
- **Template generators:** `{{uuid}}`, `{{seq}}`, `{{int:1..100}}`, plus size generators for Big-O:
  `{{n}}`, `{{n:int_array}}`, `{{n:string}}`, `{{n:object_array}}`.
- **Try it:** send one request and see the redacted response, timings and contract result.
- **Live results** every 250 ms. They survive a page reload or a dropped connection, because the
  engine keeps event history and replays it when you reconnect.

## Quick start

**You need** Rust (stable, 1.88+), Node 24 and pnpm 10.

```bash
# 1. One .env at the repo root configures both the engine and the UI.
cp .env.example .env              # then set KESTREL_TOKEN to a long random string

# 2. Engine (use --release for load tests: a debug build is a much slower load generator)
cargo run --release -p engine     # http://127.0.0.1:7070

# 3. UI
cd kestrel
pnpm install
pnpm dev                          # http://localhost:3000
```

To try everything against something with known behaviour, start the reference server and import its
spec:

```bash
cargo run --release -p engine --example target    # http://127.0.0.1:8089
```

Then in the UI, use **Import** (sidebar) and choose `engine/examples/target.openapi.yaml`. For Big-O,
set an endpoint's body to `{{n:int_array}}`, for example on `/sort` or `/linear`.

| Reference endpoint | Behaviour |
|---|---|
| `GET /fast` | Returns immediately |
| `GET /sleep?ms=20` | Sleeps exactly 20 ms |
| `GET /stall` | Server-wide 500 ms pause every 2 s (shows coordinated omission) |
| `GET /flaky` | 10% of requests return 500 |
| `GET /limited` | 429 above 50 req/s |
| `POST /linear`, `/sort`, `/quadratic` | O(n), O(n log n), O(n²) over a JSON int array |
| `POST /cached` | O(n²), cached per body: only fresh inputs expose the real cost |
| `GET /echo-headers`, `POST /echo`, `GET /redirect?to=` | For checking redaction, payloads and redirects |

## Configuration

| Variable | Default | |
|---|---|---|
| `KESTREL_TOKEN` | random per start | Required by every API call. The UI reads it at build time, so set it in `.env` |
| `KESTREL_PORT` | `7070` | The engine always binds `127.0.0.1` only |
| `KESTREL_WORKSPACE_DIR` | working directory | Where `kestrel.json` and `kestrel.secrets.json` live |
| `KESTREL_UI_ORIGINS` | `http://localhost:3000,http://127.0.0.1:3000` | Origins allowed to call the engine |
| `TARGET_PORT` | `8089` | Reference server port |

**Workspace files:**
- `kestrel.json` holds collections, endpoints and environments. It's safe to commit.
- `kestrel.secrets.json` holds secret values. It's added to `.gitignore` automatically and is never
  sent to the UI or included in reports.

## Safety

Kestrel generates load, so it's built to be hard to misuse, whether by you or by a web page in your
browser.

- **The engine binds 127.0.0.1 only.** Every request also needs the session token, a local `Host`
  header (which blocks DNS rebinding), an allowlisted `Origin`, and a JSON content type (which forces
  a CORS preflight).
- **Caps** default to 1,000 req/s, a 60 s duration, 10,000 in flight and a 5 min Big-O sweep. They
  apply to both load models, and can be raised only in local config, never through the API.
- **Load tests against anything that isn't this machine** need an explicit per-host "I own or am
  authorised to test this" confirmation. Target DNS is resolved once and pinned for the run, and
  redirects are never followed under load.
- Only load-test systems you own or have permission to test.

## Development

```bash
cargo test -p engine                 # 110 tests: API guard, runners, stats, import, contract, templates
cargo clippy -p engine --all-targets -- -D warnings
cargo fmt --all                      # rustfmt.toml: max_width 120
cargo test -p engine export_bindings # regenerate the TypeScript types in kestrel/src/types/engine

cd kestrel
pnpm lint
pnpm prettier:check
pnpm build                           # static export into kestrel/out
```

The engine's API types are the source of truth. [ts-rs](https://github.com/Aleph-Alpha/ts-rs)
generates them into `kestrel/src/types/engine/`. Don't edit those files by hand: commit what
`cargo test` produces.

**CI** (`.github/workflows/ci.yaml`) runs on pushes and PRs to `main`:
- **Engine:** rustfmt, clippy with warnings as errors, tests, and a check that the generated TypeScript
  types are committed and current.
- **UI:** Prettier, ESLint, and a production build, which includes the type check.

### Layout

```
engine/                 Rust engine (axum + tokio + reqwest)
  src/api/              HTTP API and the security guard
  src/engine/           runners: latency, load (open/closed), complexity; run registry + SSE history
  src/import/           OpenAPI 3.0/3.1 + Swagger 2.0 → collections
  src/template/         {{…}} parsing, variables, generators
  src/stats/            HDR histograms, Big-O model fitting
  src/contract.rs       response validation against spec schemas
  src/redact.rs         secret scrubbing for samples and reports
  examples/target.rs    reference server with known behaviour (+ target.openapi.yaml)
kestrel/                Next.js UI (static export)
  src/components/       run/ (controls, live charts, results), workspace/ (sidebar, editor), shared/, ui/
  src/stores/           Zustand stores: workspace (autosaved), run (live events), send
  src/types/engine/     generated from the engine; do not edit
HANDOFF.md              design notes, decisions and milestone status
```

## Deploying

Kestrel is designed to run **on your machine**, next to the engine. Hosting the UI on Vercel is easy,
since it's a static export. Hosting the engine there isn't a good fit, even though Vercel runs Rust
functions:

- **Runs outlive requests.** A run starts on one request and keeps working in the background while
  the UI streams events from another. Serverless instances can't be relied on to stay alive or share
  memory between requests, and functions stop after 300 s on Hobby or 800 s on Pro.
- **Measurements would suffer.** Load generation needs steady CPU and sockets. A shared 1–2 vCPU
  function instance adds its own jitter, and the p99 lateness numbers above wouldn't hold.
- **The security model is local.** The session token is built into the UI bundle, which is fine on
  localhost. On a public URL, anyone who loads the page gets a token for a load generator.
- **Storage is local files** (`kestrel.json`, `kestrel.secrets.json`), which don't persist on
  serverless.
- **Load-testing third parties from shared cloud infrastructure** risks the host's acceptable-use
  rules.

A hosted setup would need a long-running host for the engine (a VM or container), real user
authentication in place of the build-time token, persistent storage, and per-user limits. The planned
direction is the opposite: a single binary, with the engine serving the built UI itself (milestone M5
in `HANDOFF.md`).

## Status

Built: M0–M4. That covers the engine and API guard, latency probe, load test (both models), OpenAPI
import with contract checks, and Big-O. Next: JSON/CSV export and single-binary packaging, then
stress, spike, soak, rate-limit discovery, and Postman / curl import. See `HANDOFF.md` for the design
notes and full roadmap.
