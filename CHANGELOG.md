# Changelog

All notable changes to **Kestrel** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

No version has been tagged yet (engine and UI are both `0.1.0`), so the entries below
`[Unreleased]` are grouped by date instead of by release. When `0.1.0` is tagged, fold them into it.

## [Unreleased]

### Added

- **JSON body editor:** the body is checked as you type, with the line, column and a plain message
  for the first problem (e.g. "Line 3, column 9: Remove the trailing comma"). Templates like
  `{{seq}}` count as values. **Format** (or Shift+Alt+F) re-indents it without changing any value.
  `{`, `[`, `(` and `"` close themselves, typing a closer steps over it, Backspace removes an empty
  pair, a selection gets wrapped, and Enter keeps the indentation. Ctrl+Z undoes all of these.

- **Email verification and password reset** (accounts on, with `KESTREL_SMTP_*` set).
  - New accounts get an email with a link to confirm their address. Profile → Account shows whether
    it's verified and can send the link again.
  - **Forgot password?** on the sign-in page emails a reset link. It works once, for 30 minutes, and
    setting the new password signs the account out everywhere. Asking says the same thing whether
    or not the address has an account.
  - Links are single-use and stored only as hashes. Emails are limited to 3 per address per 15
    minutes.
  - Settings: `KESTREL_SMTP_HOST`, `KESTREL_SMTP_PORT` (default 587; 465 uses TLS),
    `KESTREL_SMTP_USERNAME`, `KESTREL_SMTP_PASSWORD`, `KESTREL_SMTP_FROM`, optional
    `KESTREL_SMTP_TLS`, and `KESTREL_PUBLIC_URL` for where links point. Without SMTP these features
    are hidden. An incomplete SMTP setup stops the engine at startup.

- **Endpoint groups by hand:** a group picker beside the endpoint name lists the collection's
  existing groups, creates a new one from what you type, or sets **No group**. Hand-made endpoints
  can now be grouped in the sidebar like imported ones.
  - **New group** under a collection's endpoints creates an empty group. Groups made by hand are
    kept while empty (`Collection.groups`, defaulting to `[]` for existing collections, so no
    migration). Each group's header can add an endpoint to it, or delete the group, which leaves its
    endpoints in the collection, ungrouped.
  - The sidebar lists ungrouped endpoints first, then groups in the order they were made.

- **Configurable caps:** `KESTREL_MAX_RPS`, `KESTREL_MAX_DURATION_S`, `KESTREL_MAX_IN_FLIGHT`,
  `KESTREL_MAX_TIMEOUT_S`, `KESTREL_MAX_SWEEP_S` and more raise or lower the safety limits (e.g.
  for soak tests). Out-of-range values stop the engine at startup; the API still can't change them.

- **Report export:** the Results card has **JSON** (the full report, secrets already redacted) and
  **CSV** buttons. The CSV holds one row per 250 ms window (requests, errors, rps, p50/p99, dropped,
  in flight, scheduler lag), or one row per input size for Big-O runs. The API is
  `GET /api/runs/:id/report?format=csv`.

- **Accounts** (`KESTREL_AUTH=on`; off by default, which keeps today's behaviour).
  - Sign-up and sign-in with email and password; the sign-in page now works, and the header shows
    the signed-in user with a sign-out menu.
  - Each account has its own workspace on any device. A browser's existing workspace is adopted by
    the first account that signs in from it, so earlier work isn't lost.
  - Passwords are hashed with Argon2id. Sessions are an HttpOnly, `SameSite=Lax` cookie lasting 30
    days from last use, and signing out ends them at once. Every API route except health and
    sign-in requires a session, including live run streams.
  - Rate limits on sign-in and sign-up (5 a minute per email, 20 per IP), with no lockout.
  - `KESTREL_SIGNUP=closed` hides sign-up; `engine user add <email>` creates an account and prints a
    generated password. `KESTREL_TRUSTED_PROXY=1` trusts proxy headers for client IPs and `Secure`
    cookies.
  - **Profile → Security:** change your password (every other device is signed out), and see where
    you're signed in, with a Sign out for each device. **Profile → Account** shows your email.
  - Passwords hashed with older parameters are upgraded at the next sign-in, and expired sessions
    are deleted daily.

- **Dashboard route group** `(dashboard)` with a shared layout (sidebar + header) for
  `/workspace`, `/profile` and `/settings`.
- **Profile page** with Account, Security, Appearance and Notifications tabs (placeholders for
  now), and a **Settings page** shell.
- **Header**: light/dark theme toggle and a user menu (Profile, Settings, Logout; not yet wired up).
- **Sign-in page** form scaffold using `react-hook-form` + `zod`, with a shared password rule
  (`config/string.ts`). It doesn't authenticate yet.
- Shared `TabPanel` component.
- **HANDOFF.md**: the self-built accounts plan (sessions, Argon2id, per-user workspaces, phases
  A1–A3), and the move to **Postgres with one schema per workspace** (phase A0). Development uses
  Postgres in Docker; production uses a provisioned database.

### Changed

- Endpoint groups are listed alphabetically, in the sidebar and the group picker (ignoring case,
  with numbers in order: `v2` before `v10`). Endpoints without a group stay at the top.
- **Sign-in page redesign:** a split screen with the form on the left and an illustrated panel on
  the right (hidden on narrow screens). Sign in and Sign up are a segmented toggle; the email and
  password fields have icons, the email shows a check once it's valid, and the password has a
  show/hide button.

- **Password rule:** 8–128 characters of anything, spaces and non-ASCII included (NIST 800-63B).
  The sign-in form used to reject spaces and non-ASCII characters while saying it required a mix.
- In `pnpm dev`, the UI calls the engine on the page's own hostname (port 7070) instead of always
  `127.0.0.1`, so the session cookie works on `localhost`.
- **Storage moved from SQLite to Postgres**, with one schema per workspace (`ws_<id>`), so one
  workspace's data can't appear in another's results even if a query forgot to filter. Every query
  runs in a transaction pinned to its workspace's schema, and an unpinned query finds no tables.
  - **Breaking:** the engine now needs `KESTREL_DATABASE_URL` and `KESTREL_SECRETS_KEY` to start.
    For development, `docker compose up -d db` starts Postgres on `localhost:5433`.
  - Secrets are encrypted at rest with AES-256-GCM, each bound to its workspace, environment and
    key.
  - `engine migrate --all` migrates every workspace schema ahead of a deploy.
  - `engine import-sqlite [dir]` moves existing `workspaces/<id>/kestrel.db` files into Postgres,
    keeping their ids so browsers keep their workspaces. Safe to re-run.
  - The Fly config no longer needs a volume; `docker-compose.yml` now includes Postgres.
- **Run history moved into the sidebar** as a Collections / History tab, instead of a sheet opened
  from the header. Runs are fetched only while the tab is visible, and opening a run no longer
  closes the panel.
- `/workspace` moved under the dashboard layout. The old standalone `app/workspace/page.tsx` was
  removed.
- Fonts are now DM Sans and JetBrains Mono (were Space Grotesk and Space Mono). `next-themes` uses
  the `class` attribute, so dark mode applies.
- The app background uses the muted token. The selected endpoint is highlighted in the primary
  tint.
- Engine sources reformatted with rustfmt; no behaviour change.

### Fixed

- No more hydration warning on `<html>` in development: the theme class that next-themes adds
  before React loads is now expected.
- Two engine tests that no longer compiled, and a content-type test that passed for the wrong
  reason. CI now runs the engine tests against a Postgres service.
- **Request tabs** could stay squashed after closing tabs: they didn't grow back when few remained,
  and sometimes showed only the close button. Held widths are now released when a tab opens or the
  pointer leaves the strip, and are never held while the strip is scrolling.
- **Workspace page**: the status bar and the Run button were cut off below the screen. The page now
  fills the space under the header, and the main column and Run panel scroll on their own.
- Profile tabs pointed at panel ids that didn't exist, so assistive tech couldn't link tabs to
  panels.

## 2026-09-30

### Added

- **Workspace isolation**: each browser gets its own workspace (a random id in `localStorage`, sent
  as `X-Kestrel-Workspace`, or `?workspace=` for SSE). Each workspace has its own SQLite database
  under `workspaces/<id>/`, and runs are scoped to the workspace that started them.

### Fixed

- `KESTREL_ALLOWED_HOSTS` accepts pasted URLs (`https://host/`): the scheme and trailing slashes
  are stripped.
- Requests without a body are handled in the Run controls. Test labels made consistent.
- The response view's copy button is positioned so it stays reachable.

## 2026-09-29

### Added

- **Response extract rules**: save values from a Send response into variables or secrets.
- `application/x-www-form-urlencoded` and `multipart/form-data` request bodies, with a file upload
  API for multipart file fields.
- **Single binary**: the Next.js UI is embedded in the engine and served with the session token
  injected into the page.
- **Deployment**: Dockerfile, Fly.io config and `docker-compose.yml`.
- **Browser-style request tabs** for several open endpoints.
- **Run history**: list and reopen past runs.
- Auth welcome layout (placeholder).
- Toast notifications, an animated sidebar and loaders.
- `KESTREL_ALLOWED_HOSTS` for the hostname a platform serves the app on (its `http` and `https`
  origins are allowed too), with a clear warning when a request is rejected for its `Host`.

### Changed

- **Storage moved to SQLite** (`kestrel.db`) from `kestrel.json`, `kestrel.secrets.json` and
  `kestrel-files/`. Existing files are imported on first start.
- Run reports are saved and survive engine restarts. Confirmed hosts are saved instead of held in
  memory.
- The sample workspace uses template variables instead of hard-coded credentials.
- UI polish: consistent focus colour, icons, spacing, dialog widths and sheet animations.

### Fixed

- Request compilation, redaction and CORS handling.

## 2026-09-28

### Added

- **Big-O test (M4)**: size generators, shuffled-round sweeps, AICc model fitting, confidence
  labels, the log-log slope, and slow-limit / time-budget stops. The UI has a live log-log chart
  and a verdict view.
- **Spec import (M3)**: OpenAPI 3.0 / 3.1 and Swagger 2.0, as JSON or YAML, from text, a file or a
  URL. Each spec becomes one collection.
- **Contract checks** on Send, latency runs and load runs: undeclared status codes and schema
  violations are flagged, and sampled under load.
- **Collections**: endpoints are grouped into collections with default variables (create, rename,
  delete, configure).
- CI workflow for linting, formatting, testing and building the engine and the UI. rustfmt config.
- Shared UI components: method badge, status badge, stat tile, status bar, collection list.

### Changed

- The UI uses shadcn semantic design tokens. The main layout, environment bar and response view
  were reworked.
- The reference target's default port moved to 8089 (8080 is often taken).

## 2026-09-27

### Added

- **Initial release of Kestrel (M0–M2)**: a Rust engine (axum) and a Next.js UI for
  performance-testing HTTP endpoints.
  - An engine API guard: session token, `Host` / `Origin` allowlist, JSON-only bodies.
  - Live run events over SSE, with event ids, history replay and resume.
  - Workspace, environments and secrets; `{{var}}` templating with generators; the `/send` try-it
    button.
  - **Latency probe**: all / ok / TTFB percentiles, histogram, cold request, error classes,
    redacted samples.
  - **Load test**: closed and open models, an in-flight cap with dropped-send counting,
    scheduler-lag tracking, coordinated-omission-correct timing, and host confirmation.
  - DNS pinning and the redirect policy.
  - The reference test target (`cargo run -p engine --example target`).
  - TypeScript types generated from Rust with `ts-rs`.
