# Changelog

All notable changes to **Kestrel** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

No version has been tagged yet (engine and UI are both `0.1.0`), so the entries below
`[Unreleased]` are grouped by date instead of by release. When `0.1.0` is tagged, fold them into it.

## [Unreleased]

### Added

- **Dashboard route group** `(dashboard)` with a shared layout (sidebar + header) for
  `/workspace`, `/profile` and `/settings`.
- **Profile page** with Account, Security, Appearance and Notifications tabs (placeholders for
  now), and a **Settings page** shell.
- **Header**: light/dark theme toggle and a user menu (Profile, Settings, Logout; not yet wired up).
- **Sign-in page** form scaffold using `react-hook-form` + `zod`, with a shared password rule
  (`config/string.ts`). It doesn't authenticate yet.
- Shared `TabPanel` component.
- `DATABASE_URL` placeholder in `.env.example`, ahead of the Postgres move.
- **HANDOFF.md**: the self-built accounts plan (sessions, Argon2id, per-user workspaces, phases
  A1–A3), and the move to **Postgres with one schema per workspace** (phase A0). Development uses
  Postgres in Docker; production uses a provisioned database.

### Changed

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
