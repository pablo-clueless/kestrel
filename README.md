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
  its secrets, then the collection. Secrets are write-only in the UI, encrypted at rest, and
  redacted from every response and report, including when a server echoes them back.
- **Template generators:** `{{uuid}}`, `{{seq}}`, `{{int:1..100}}`, plus size generators for Big-O:
  `{{n}}`, `{{n:int_array}}`, `{{n:string}}`, `{{n:object_array}}`.
- **Try it:** send one request and see the redacted response, timings and contract result.
- **Live results** every 250 ms. They survive a page reload or a dropped connection, because the
  engine keeps event history and replays it when you reconnect.

## Quick start

**You need** Rust (stable, 1.88+), Node 24, pnpm 10 and Docker (for Postgres).

```bash
# 1. One .env at the repo root configures both the engine and the UI.
cp .env.example .env              # then set KESTREL_TOKEN to a long random string,
                                  # and KESTREL_SECRETS_KEY to the output of `openssl rand -hex 32`

# 2. Postgres, on localhost:5433 (KESTREL_DATABASE_URL in .env.example already points at it)
docker compose up -d db

# 3. Engine (use --release for load tests: a debug build is a much slower load generator)
cargo run --release -p engine     # http://127.0.0.1:7070

# 4. UI
cd kestrel
pnpm install
pnpm dev                          # http://localhost:3000
```

**Single binary:** the engine also serves the built UI. Run `pnpm build` in `kestrel/`, then
`cargo run --release -p engine` and open http://localhost:7070. Release builds embed the UI, so the
binary plus a database is the whole app. The engine injects the session token into the page it
serves, so `KESTREL_TOKEN` isn't needed this way (the database URL and secrets key still are).

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
| `KESTREL_DATABASE_URL` | – (required) | Postgres. Development: `postgres://kestrel:kestrel@localhost:5433/kestrel` from `docker compose up -d db` |
| `KESTREL_SECRETS_KEY` | – (required) | 64 hex characters (`openssl rand -hex 32`) that encrypt stored secrets. Keep it safe and stable: without it, stored secrets can't be read |
| `KESTREL_DB_MAX_CONNECTIONS` | `10` | Database connection pool size |
| `KESTREL_AUTH` | `off` | `on` requires signing in, and each account gets its own workspace. See Accounts below |
| `KESTREL_SIGNUP` | `open` | `closed` hides sign-up; create accounts with `engine user add <email>` |
| `KESTREL_TRUSTED_PROXY` | `0` | `1` only behind a proxy that sets `Fly-Client-IP` / `X-Forwarded-For` / `X-Forwarded-Proto`: used for per-IP sign-in limits and `Secure` cookies |
| `KESTREL_TOKEN` | random per start | Required by every API call. The UI reads it at build time, so set it in `.env` |
| `KESTREL_PORT` | `7070` | |
| `KESTREL_BIND` | `127.0.0.1` | Loopback by default. Only change it on a private network (see Deploying) |
| `KESTREL_WORKSPACE_DIR` | working directory | Only read by `engine import-sqlite`, which looks for old SQLite workspaces under `workspaces/` |
| `KESTREL_UI_ORIGINS` | `http://localhost:3000,http://127.0.0.1:3000` | Origins allowed to call the engine |
| `TARGET_PORT` | `8089` | Reference server port |

**Workspaces, accounts off** (`KESTREL_AUTH=off`, the default): each browser gets its own
workspace. The UI makes up a random id on first load, keeps it in `localStorage` and sends it as
`X-Kestrel-Workspace` with every call, so people sharing one engine don't see each other's requests,
secrets or runs. Clearing site data starts a new, empty workspace, and anyone who learns an id can
open that workspace. Fine on your own machine or a private network.

**Accounts** (`KESTREL_AUTH=on`): the UI asks people to sign in or create an account (email and a
password of 8–128 characters), and each account has its own workspace, on any device. A browser's
existing workspace is adopted by the first account that signs in from it, so work done before
signing up isn't lost.
- Passwords are hashed with Argon2id. Sessions are an HttpOnly, `SameSite=Lax` cookie that lasts 30
  days from last use; signing out ends the session at once.
- Sign-in and sign-up are rate limited (5 attempts a minute per email, 20 per IP), with no lockout.
- For a deployment reachable from the internet, also set `KESTREL_SIGNUP=closed` and create accounts
  with `engine user add <email>` (it prints a generated password once). With open sign-up, anyone
  could create an account and use your engine to send load.
- **Profile → Security** changes the password (which signs out every other device) and lists where
  the account is signed in, with a Sign out for each. Expired sessions are cleaned up daily.
  Email verification and password reset by email arrive later (see `HANDOFF.md` → Accounts).
- In `pnpm dev`, open the UI on `localhost:3000`, not `127.0.0.1:3000`: the cookie only crosses
  between the UI and the engine when both are on the same hostname.

**Storage:** Postgres, with **one schema per workspace** (`ws_<id>`), so one workspace's data can't
show up in another's results even if a query forgot to filter. A shared `auth` schema holds the
registry of workspaces. Each workspace schema holds:
- collections, endpoints and environments;
- secret values, encrypted with `KESTREL_SECRETS_KEY` and never sent to the UI or included in reports;
- files uploaded for multipart bodies;
- the reports of the last 500 finished runs;
- hosts confirmed for load testing.

Schemas are created and migrated when a workspace is first opened. Before deploying a new engine
version, `engine migrate --all` brings every workspace up to date ahead of time. Back up with
`pg_dump` as for any Postgres database.

**Upgrading from SQLite:** run `engine import-sqlite` once with the old data directory available
(`KESTREL_WORKSPACE_DIR`, or pass the `workspaces/` directory). Each `workspaces/<id>/kestrel.db`
becomes `ws_<id>` with the same id, so every browser keeps its workspace. It's safe to run again: it
skips workspaces that are already in Postgres.

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
docker compose up -d db              # the store and API tests need Postgres (KESTREL_TEST_DATABASE_URL)
cargo test -p engine                 # 165 tests: API guard, accounts, store isolation, runners, stats, …
cargo clippy -p engine --all-targets -- -D warnings
cargo fmt --all                      # rustfmt.toml: max_width 120
cargo test -p engine export_bindings # regenerate the TypeScript types in kestrel/src/types/engine

cd kestrel
pnpm lint
pnpm prettier:check
pnpm build                           # static export into kestrel/out
```

Each database test works in its own uniquely prefixed schemas and drops them when it ends, so tests
run in parallel against one database (the development one is fine). Without
`KESTREL_TEST_DATABASE_URL` they're skipped with a note; in CI, where `CI` is set, they fail instead.

The engine's API types are the source of truth. [ts-rs](https://github.com/Aleph-Alpha/ts-rs)
generates them into `kestrel/src/types/engine/`. Don't edit those files by hand: commit what
`cargo test` produces.

**CI** (`.github/workflows/ci.yaml`) runs on pushes and PRs to `main`:
- **Engine:** rustfmt, clippy with warnings as errors, tests (against a Postgres service container),
  and a check that the generated TypeScript types are committed and current.
- **UI:** Prettier, ESLint, and a production build, which includes the type check.

### Layout

```
engine/                 Rust engine (axum + tokio + reqwest)
  src/api/              HTTP API and the security guard
  src/db/               Postgres: pool, per-workspace schemas + migrations, secrets encryption, SQLite import
  src/model/            workspace types, the per-workspace store and its cache
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

One container runs the engine binary with the UI embedded, serving both on port 7070. It keeps
nothing itself: all data is in Postgres.

```bash
docker compose up -d --build              # Postgres + the engine: http://localhost:7070
# or without compose, against a database you run:
docker build -t kestrel .
docker run --rm -p 127.0.0.1:7070:7070 \
  -e KESTREL_DATABASE_URL=postgres://… -e KESTREL_SECRETS_KEY=… kestrel
```

With compose, the data is on the `kestrel-db` volume; set `KESTREL_SECRETS_KEY` in `.env` first.
Containers can be rebuilt and recreated freely. Only remove the volume if you mean to wipe every
workspace. Back up with `docker compose exec db pg_dump -U kestrel kestrel > kestrel-backup.sql`.

> **Accounts are off by default.** Then anyone who can load the UI can drive the load generator:
> publish the port on `127.0.0.1` (as above) or keep it on a private network. To expose it, set
> `KESTREL_AUTH=on` and `KESTREL_SIGNUP=closed` (see Accounts above).

### Fly.io (private)

`fly.toml` deploys one machine with **no public IP**. You open it through `fly proxy` over Fly's
WireGuard, so only members of your Fly org can reach it.

```bash
fly apps create <your-app-name>                    # and set `app` in fly.toml to match
fly secrets set KESTREL_DATABASE_URL='postgres://…?sslmode=require' KESTREL_SECRETS_KEY=$(openssl rand -hex 32)
fly deploy --no-public-ips --ha=false              # exactly one machine: live runs are held in its memory
fly proxy 7070:7070                                # then open http://localhost:7070
```

- Workspaces, secrets and run history live in the Postgres database you provide. Any provider
  works; if it puts a connection pooler in front, use its **transaction** mode.
- Store `KESTREL_SECRETS_KEY` somewhere besides Fly too: losing it means losing every stored secret.
- Upgrading a deployment that used SQLite: deploy, then run
  `fly ssh console -C "kestrel-engine import-sqlite /data/workspaces"` once, while the old
  `kestrel_data` volume is still mounted.
- It's a dedicated-CPU machine because shared CPU adds jitter to latency numbers. Switch `cpu_kind`
  to `shared` in `fly.toml` for lighter, cheaper use.
- Latency is measured from the Fly region, not from your machine.
- To use a different local port (`fly proxy 8000:7070`), set `KESTREL_ALLOWED_HOSTS=localhost:8000`,
  since the engine checks the `Host` header.
- Only load-test APIs you own or have permission to test. That's also Fly's rule.

| Variable | Container default | |
|---|---|---|
| `KESTREL_BIND` | `::` | Any address, IPv4 and IPv6 (Fly's private network is IPv6). Defaults to `127.0.0.1` outside the container |
| `KESTREL_WORKSPACE_DIR` | `/data` | Only for `import-sqlite`: where an old SQLite volume is mounted |
| `KESTREL_ALLOWED_HOSTS` | – | Extra `Host` values to accept, comma-separated, e.g. the hostname your platform serves the app on (`kestrel.example.com`, no scheme). Their `http://` and `https://` origins are allowed too |

`GET /healthz` answers `ok` without a token, for health checks.

### Why not Vercel

Hosting the UI on Vercel is easy, since it's a static export. Hosting the engine there isn't a good
fit, even though Vercel runs Rust functions:

- **Runs outlive requests.** A run starts on one request and keeps working in the background while
  the UI streams events from another. Serverless instances can't be relied on to stay alive or share
  memory between requests, and functions stop after 300 s on Hobby or 800 s on Pro.
- **Measurements would suffer.** Load generation needs steady CPU and sockets. A shared 1–2 vCPU
  function instance adds its own jitter, and the p99 lateness numbers above wouldn't hold.
- **The deployment token alone is a local security model.** It's built into the UI bundle, which is
  fine on localhost; on a public URL anyone who loads the page has it. Accounts
  (`KESTREL_AUTH=on`) close that gap.
- **The engine holds live runs in memory**, so it needs one long-lived process. (Storage is no
  longer the obstacle: it's Postgres.)
- **Load-testing third parties from shared cloud infrastructure** risks the host's acceptable-use
  rules.

The container above solves all of these. A **public** deployment should run with
`KESTREL_AUTH=on` and `KESTREL_SIGNUP=closed`.

## Status

Built: M0–M4, plus Postgres storage (one schema per workspace) and accounts (sign-up, sign-in,
sessions). That covers the engine and API guard, latency probe, load test (both models), OpenAPI
import with contract checks, Big-O, and single-binary / container packaging (UI served by the
engine, private Fly.io deploy). Next: password changes and session management, JSON/CSV export, then
stress, spike, soak, rate-limit discovery, and Postman / curl import. See `HANDOFF.md` for the design
notes and full roadmap.
