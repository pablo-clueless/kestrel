![Kestrel screenshot](./kestrel/public/assets/logo.png)

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
| `KESTREL_SMTP_HOST` | – (email off) | SMTP server for verification and password-reset emails. Unset turns both off |
| `KESTREL_SMTP_PORT` | `587` | `465` means TLS from the start; other ports upgrade with STARTTLS |
| `KESTREL_SMTP_TLS` | by port | `tls`, `starttls`, or `none` (only for a local catcher like Mailpit) |
| `KESTREL_SMTP_USERNAME`, `KESTREL_SMTP_PASSWORD` | – | Set both or neither |
| `KESTREL_SMTP_FROM` | – (required with a host) | Sender, e.g. `Kestrel <no-reply@example.com>` |
| `KESTREL_PUBLIC_URL` | the page that asked | Where links in emails point, e.g. `https://kestrel.example.com` |
| `KESTREL_TOKEN` | random per start | Required by every API call. The UI reads it at build time, so set it in `.env` |
| `KESTREL_PORT` | `7070` | |
| `KESTREL_BIND` | `127.0.0.1` | Loopback by default. Only change it on a private network (see Deploying) |
| `KESTREL_WORKSPACE_DIR` | working directory | Only read by `engine import-sqlite`, which looks for old SQLite workspaces under `workspaces/` |
| `KESTREL_UI_ORIGINS` | `http://localhost:3000,http://127.0.0.1:3000` | Origins allowed to call the engine |
| `TARGET_PORT` | `8089` | Reference server port |
| `KESTREL_MAX_RPS` | `1000` | Highest request rate (open model), and the pace for closed-model users |
| `KESTREL_MAX_DURATION_S` | `60` | Longest load or latency run, in seconds (up to 7 days; raise it for soak tests) |
| `KESTREL_MAX_IN_FLIGHT` | `10000` | Most requests in flight, and most closed-model users |
| `KESTREL_MAX_TIMEOUT_S` | `60` | Longest per-request timeout, in seconds |
| `KESTREL_MAX_SWEEP_S` | `300` | Longest Big-O sweep, in seconds |
| `KESTREL_MAX_SAMPLES`, `KESTREL_MAX_WARMUP`, `KESTREL_MAX_N`, `KESTREL_MAX_POINTS` | `10000`, `1000`, `1000000`, `40` | Latency/Big-O sample counts, the largest Big-O `n`, and the most Big-O sizes |

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
- **Email** (`KESTREL_SMTP_*`): new accounts get a link to confirm their address (Profile → Account
  can send it again), and the sign-in page gets **Forgot password?**. Reset links work once, for 30
  minutes, and setting the new password signs the account out everywhere. Asking for a reset says
  the same thing whether or not the address has an account. Without SMTP both features are hidden.
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
  apply to both load models. Whoever runs the engine can raise or lower them with `KESTREL_MAX_*`
  variables (see Configuration); the API never can.
- **Load tests against anything that isn't this machine** need an explicit per-host "I own or am
  authorised to test this" confirmation. Target DNS is resolved once and pinned for the run, and
  redirects are never followed under load.
- Only load-test systems you own or have permission to test.
