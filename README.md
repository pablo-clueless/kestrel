![Kestrel](./kestrel/public/assets/logo.png)

# Kestrel

Kestrel is a tool for performance-testing HTTP APIs. Add an endpoint, or import a whole API from an
OpenAPI spec, Postman collection, HAR file or curl commands. Then measure how fast it responds, how it
holds up under load, how it handles concurrent writes, and how its latency grows with input size.
While it runs, it checks responses against your spec.

It has two parts. A **Rust engine** sends the requests and takes the measurements, and a **web UI**
sets up tests and charts the results live. The browser never sends test traffic itself, so the
numbers aren't limited by what a browser can do.

**Try it online:** [Kestrel](https://kestrel-oqjm.onrender.com), or [run it yourself](#getting-started).

```
 Browser (UI) ──▶ Kestrel engine ──HTTP──▶ the API under test
      ▲                 │
      └─ live results ──┘
```

## What it measures

| Test | What it tells you |
|---|---|
| **Latency probe** | How fast one request is. Reports p50/p90/p99/max, time to first byte, a distribution, and the first (cold) request on its own. |
| **Load test** | How the API behaves under sustained traffic. Choose a fixed request rate (open model) or a fixed number of users (closed model). Latency is measured from when each request *should* have gone out, so a stalled server can't hide its own slowness. |
| **Spike** | What happens during a sudden burst, and how long the API takes to recover once it ends. |
| **Breakpoint** | The highest rate the API sustains before errors or p99 latency pass the limits you set. |
| **Rate-limit discovery** | The rate at which the API starts returning 429s, and the rate-limit headers it sends. |
| **Concurrency (race)** | What happens when the same request arrives at the same moment several times, for example duplicate POSTs getting through or conflicting writes going unrejected. |
| **Big-O** | Whether latency grows like O(1), O(n), O(n log n) or O(n²) as the input gets bigger. It gives a verdict with a confidence level, and says "inconclusive" rather than guessing. |
| **Spec contract** | Whether responses match the spec: undeclared status codes and JSON Schema mismatches, with where the mismatch is. |

Latency and load results also show **where the time went**: DNS, connecting (TCP + TLS), waiting for
the server, and downloading the response.

## Features

- **Collections** of endpoints, one per API, with groups, shared variables and shared headers.
- **Import** from OpenAPI 3.0 / 3.1 or Swagger 2.0 (JSON or YAML), Postman collections (v2.0 / v2.1),
  HAR files, or curl commands. Specs can come from a file, pasted text or a URL. Pasting a curl
  command into the URL field fills in that request.
- **Environments and secrets.** `{{variables}}` resolve from the active environment, then its
  secrets, then the collection. Secrets are encrypted at rest, can't be read back in the UI, and are
  redacted from responses and reports.
- **Template generators** for realistic input: `{{uuid}}`, `{{seq}}`, `{{int:1..100}}`, plus the
  sized inputs Big-O tests use: `{{n}}`, `{{n:int_array}}`, `{{n:string}}`, `{{n:object_array}}`.
- **Try it:** send one request and see the response, timings and contract check. Save values from
  the response into variables, or download the body.
- **Live results** that stream in while a test runs and survive a page reload.
- **Run history and export:** reopen past runs, or export a report as JSON.
- **Optional accounts**, with sign-in, per-user workspaces, email verification and password reset.

## Getting started

**Requirements:** Rust 1.88+, Node 24, pnpm 10, and Docker (for Postgres).

```bash
# 1. Configuration: one .env at the repo root is shared by the engine and the UI.
cp .env.example .env
#    Set KESTREL_SECRETS_KEY to the output of `openssl rand -hex 32`,
#    and KESTREL_TOKEN to a long random string.

# 2. Database
docker compose up -d db

# 3. Engine (use --release for load tests; a debug build is much slower)
cargo run --release -p engine        # http://127.0.0.1:7070

# 4. UI
cd kestrel
pnpm install
pnpm dev                             # http://localhost:3000
```

### Single binary

A release build of the engine serves the UI too. Run `pnpm build` in `kestrel/`, then
`cargo run --release -p engine`, and open http://localhost:7070. The binary and a database are all
you need.

### Docker

```bash
docker compose up -d --build         # Postgres + Kestrel at http://localhost:7070
```

`KESTREL_SECRETS_KEY` is read from `.env`.

## Try it on the reference server

Kestrel ships with a small server whose behaviour is known in advance, which is useful for seeing what
each test reports:

```bash
cargo run --release -p engine --example target    # http://127.0.0.1:8089
```

In the UI, open **Import** and choose `engine/examples/target.openapi.yaml`. To try Big-O, set the
body of `/sort` or `/linear` to `{{n:int_array}}`.

| Endpoint | Behaviour |
|---|---|
| `GET /fast` | Returns immediately |
| `GET /sleep?ms=20` | Takes 20 ms |
| `GET /stall` | Pauses the whole server for 500 ms every 2 s |
| `GET /flaky` | Fails 10% of requests with a 500 |
| `GET /limited` | Returns 429 above 50 req/s |
| `POST /linear`, `/sort`, `/quadratic` | O(n), O(n log n) and O(n²) work over a JSON int array |
| `POST /cached` | O(n²), cached per input, so only fresh inputs show the real cost |
| `GET /echo-headers`, `POST /echo`, `GET /redirect?to=` | For checking redaction, payloads and redirects |

## Configuration

Kestrel reads its settings from environment variables. `.env.example` lists all of them with
explanations. The main ones:

| Variable | Default | Purpose |
|---|---|---|
| `KESTREL_DATABASE_URL` | required | Postgres connection URL |
| `KESTREL_SECRETS_KEY` | required | Key that encrypts stored secrets. Keep it safe and don't change it, or stored secrets can't be read. |
| `KESTREL_TOKEN` | random per start | Session token the UI uses to talk to the engine. Set it when running the UI with `pnpm dev`. |
| `KESTREL_PORT` | `7070` | Engine port (falls back to `PORT`, as set by Render, before `7070`) |
| `KESTREL_AUTH` | `off` | `on` requires sign-in, gives each account its own workspace, and lets owners invite others as editors or viewers |
| `KESTREL_SIGNUP` | `open` | `closed` turns off self sign-up; create accounts with `engine user add <email>` |
| `KESTREL_SMTP_*` | unset | Email for address verification, password reset and workspace invites (accounts only) |
| `KESTREL_MAX_*` | see `.env.example` | Safety caps on request rate, run length, concurrency and Big-O sweeps |

## Responsible use

Kestrel generates real load, so it ships with guardrails:

- By default the engine only accepts connections from this machine.
- Request rate, run length and concurrency are capped. Only whoever runs the engine can change the
  caps.
- Before you load-test or race-test a host that isn't this machine, you have to confirm you own it
  or are authorised to test it.
- If you expose Kestrel beyond your own machine, turn accounts on (`KESTREL_AUTH=on`) and close
  sign-up.

**Only test systems you own or have permission to test.**
