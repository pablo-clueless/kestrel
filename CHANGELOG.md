# Changelog

All notable changes to **Kestrel** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

No version has been tagged yet (engine and UI are both `0.1.0`), so the entries below
`[Unreleased]` are grouped by date instead of by release. When `0.1.0` is tagged, fold them into it.

## [Unreleased]

### Added

- **Token refresh during load tests.** Switch on **Keep a token fresh**, choose the request that
  gets a token and how often to fetch one (every 5 minutes by default). Long runs, soaks above all,
  no longer end in a wall of 401s when a token expires.
  - **The token request:** any endpoint with an On Response rule that saves the token, the same
    rules Send uses.
  - **When it's sent:** before the first request, on the schedule, and straight away when the run
    starts getting 401s (at most every 5 seconds).
  - **During the run:** requests switch to the new token without the run stopping.
  - **Where the token is kept:** a token saved as a secret is stored, so the next run starts with
    it. One saved as a variable lasts for this run only, because variables are the UI's to save.
  - **Report notes:** how many refreshes there were, how many were early because of 401s, and any
    failures. If a refresh fails, the run keeps the token it had.
  - **Validation:** the run is refused up front if the token request has no On Response rules, if
    there's no environment to save into, or if the interval is under 10 seconds.
- **Compare two runs side by side.** In **History**, choose **Compare**, tick two finished runs and
  open the comparison. The older run is "before".
  - **Metrics:** requests, error rate, throughput, p50/p90/p99/max/mean and TTFB p50, with each
    change marked better or worse. Changes under 5% (or under 1 ms for latencies) show as ≈, since
    they're within run-to-run noise.
  - **Status codes:** listed for both runs.
  - **Key results by test type:** the Big-O verdict and slope, payload MB/s and fixed cost, where a
    breakpoint run broke, where 429s started, spike recovery, and the main finding of race, timeout
    and soak runs.
  - **Like-for-like warning:** shown when the runs differ in test type, endpoint, settings or target
    address.
- **A warning for very short secrets.** Saving a secret of 1 or 2 characters now shows a warning
  toast. Values that short aren't hidden in response bodies, because they'd match ordinary text, but
  they're still hidden in Authorization, Cookie and API-key headers. This covers the environment
  editor and saving a value from a response. The rule itself is unchanged and is now documented
  where it's defined.
- **OpenAPI and Swagger specs split across files import whole.** External `$ref`s, such as
  `models.yaml#/Pet` or `https://…/common.json#/Error`, are fetched and bundled into the spec
  before importing.
  - **What it fixes:** request examples and contract checks now see those schemas, where before
    they were silently missing.
  - **References stay references:** recursive schemas still work, and refs inside a fetched file
    resolve against that file.
  - **Relative refs** need the spec's own address, so they work when you import by URL. Absolute
    http(s) refs work from pasted text and uploaded files too.
  - **Limits:** up to 20 documents and 10 MB in total.
  - **Warnings:** anything that couldn't be followed is listed in the review step, for example a
    relative ref in pasted text or a document that failed to load.
- **Cookie parameters are imported.** A spec's cookie parameters become the endpoint's `Cookie`
  header, using the spec's example values; before, they were skipped with a warning. Required
  cookies are sent; if none are required, they're all listed and the header starts switched off. A
  cookie API key goes in the same header as `name={{apiKey}}`, where before it was wrongly imported
  as a header API key.
- **Payload baseline for Big-O and payload scaling.** Pick an endpoint under **Payload baseline
  (echo)**, for example one that returns the body it's sent. It must be on the same host.
  - **How it runs:** every sample's exact body and headers are also sent there, straight after the
    real request, so both see the same conditions.
  - **Results:** each size gets an echo median, shown as an "echo baseline" line on the chart and an
    **Echo** column in the table.
  - **The note:** it says how much of the endpoint's growth the echo shows too, which is moving and
    parsing the bytes. The rest is the endpoint's own work.
  - **Counting:** baseline requests aren't counted as the endpoint's. Failed ones are left out of
    the curve and noted.
- **Multi-endpoint load tests.** Any load test (open, closed, breakpoint, spike, rate limit, soak) can
  mix in other endpoints from the same collection: switch on **Mix with other endpoints**, tick
  endpoints and give each a weight.
  - **Sending:** each request goes to one endpoint, chosen by weight. Weights 3 and 1 send three
    quarters of the requests to the first.
  - **Results:** a table shows each endpoint's share of requests, error rate, p50, p99 and status
    codes. The rest of the report is the combined traffic.
  - **Contract checks:** each endpoint keeps its own.
  - **Limits:** 2 to 20 endpoints, weights from 1 to 1,000. They must all be on the run's host,
    because a run pins one host's address. An endpoint on another host is refused before the run
    starts, with a message naming it.
- **Payload scaling test** ("Payload scaling" in the run panel). It runs Big-O's sweep with the size
  in bytes (1 kB to 1 MB by default) and reads the results as bytes.
  - **Sizing the payload:** use `{{n:string}}` in the body, or `{{n}}` in a parameter that grows the
    response, such as a page size. Unlike Big-O, it works for GET requests too.
  - **Fixed cost and throughput:** a straight line through latency against the bytes sent and
    received gives what a request costs whatever its size, and the effective MB/s for the bytes on
    top.
  - **Per size:** each size gets its own MB/s, and the results show the best one.
  - **Findings:**
    - Latency growing faster than the payload is flagged (Big-O's log-log slope above 1.3), as is
      throughput that peaks and then falls.
    - If the payload barely changed with n, it says so.
    - On a target on this machine, it notes that MB/s measures CPU and memory copies, not a network.
  - **Results:** the payload findings, three tiles (fixed cost, effective throughput, best at one
    size) and a per-size table appear above the usual Big-O results.
  - **Limits:** sizes above 1,000,000 need `KESTREL_MAX_N` raised.
- **Soak test** (Load test → "Soak: steady load for a long time"). It holds a steady rate for minutes
  or hours, with the duration set in minutes, then reads the run for drift.
  - **Windows:** the run is split into windows sized to its length, from 1 s for a one-minute run to
    60 s for runs of two hours or more. That keeps the engine's memory flat however long the run is.
  - **Timeline:** the windows become the report's timeline, so the chart and CSV cover the whole run.
    The live history only holds about 17 minutes.
  - **Drift:** after a warm-up tenth, the early fifth of the run is compared with the last fifth.
    - Latency that grew by more than a quarter (plus 5 ms) points at a leak, a queue or a filling
      pool.
    - Errors that rose by more than a point are flagged.
    - Otherwise the run is reported as steady.
  - **Expiring credentials:** 401s or 403s that take over partway through are called out as
    credentials that probably expired.
  - **Before you start:** the run panel warns when the request sends credentials, since a token that
    expires mid-run shows up as a wall of 401s. When the soak is longer than the engine's duration cap,
    it explains how to raise `KESTREL_MAX_DURATION_S` (up to 7 days).
- **Timeout behaviour test** ("Timeout behaviour" in the run panel). It answers whether the server
  fails fast or hangs, and whether requests the client gives up on slow down the ones after them. It
  runs in three phases:
  1. **Before:** probes sent one at a time, with the normal timeout.
  2. **Burst:** requests the client abandons mid-flight, 100 by default, 20 at a time. The give-up
     time defaults to a quarter of the probes' median time.
  3. **After:** the same probes again.

  What it reports:
  - **Hung:** probes that reached the timeout are flagged, with a suggestion to answer 503 or 504
    after a set time instead.
  - **Failing fast:** quick 5xx responses or connection failures are reported as good.
  - **Abandoned work:** if probes are slower after the burst, the report gives p50 and p99 before and
    after and how long recovery took. It says the server probably keeps working on requests nobody is
    waiting for.
  - **Nothing abandoned:** if every burst request was answered in time anyway, it says the burst tested
    nothing.

  Like a load test, it needs the host confirmed unless the target is this machine.
- **Concurrency test** ("Concurrency (race)" in the run panel). It sends the same request several
  times at once (2 to 1,000 per round, 1 to 50 rounds, with a pause between), then explains what came
  back.
  - **How it sends:** every request in a round waits on one barrier, so they reach the server
    together. The request is rendered once per round, so `{{uuid}}` and `{{seq}}` change between
    rounds but not within one.
  - **Each round shows:** the status codes, how many succeeded, how many distinct bodies the
    successes returned, and how long all the requests were in flight at once.
  - **Findings depend on the method:**
    - 5xx responses are a problem for any method.
    - Several successful POSTs mean duplicates got through, and differing bodies mean separate
      records were made. One success with the rest refused (409, 422 and similar) is good.
    - PUT and PATCH are good when some writes are refused with 409 or 412.
    - DELETE is good with one success and the rest 404 or 410.
    - A GET is flagged as a read.
  - **Also flagged:** rounds where the requests didn't actually overlap, and outcomes that changed
    from round to round.
  - **Errors:** only 5xx responses and requests with no answer count. A 409 is often the right
    answer here.
  - **Safety:** like a load test, it needs the host confirmed unless the target is this machine.
- **HAR import.** Drop a `.har` file (saved from a browser's dev tools, or from a proxy) into
  **Import**, and its API calls become a collection.
  - **Skipped:** page assets (scripts, styles, images, fonts, media), CORS preflights, non-HTTP URLs
    (`data:`, `blob:`, `ws:`), and repeats of the same method, URL and body (polling). The review
    step says how many of each were left out.
  - **Headers:** ones the browser or client sets itself (`:authority`, `sec-fetch-*`, `sec-ch-*`,
    `Host`, `Content-Length`, `Accept-Encoding`…) are dropped.
  - **Requests:** an `Authorization` header becomes Bearer or Basic auth. Bodies become JSON, form,
    multipart or raw. File fields need their files attached again.
  - **Collection:** the most common origin becomes `base`. With several hosts, endpoints are grouped
    by host.
  - **Upload:** the UI strips response bodies and initiator stack traces before uploading, since they
    are most of a HAR's size.
- **Collection headers.** Collection settings now has **Variables** and **Headers** tabs. Headers
  set there are sent with every endpoint in the collection, on Send and in runs.
  - Values can use `{{variables}}`.
  - Collection headers are sent before the endpoint's own.
  - If an endpoint has its own enabled header with the same name (in any case), or its auth sets
    `Authorization`, the endpoint's version wins.
  - An endpoint's Headers tab lists the collection headers it will also send.
  - Collections are stored as JSON, so existing ones need no migration; they just start with no
    headers.
- **Download a response body.** The response view has **Copy** and **Download JSON** buttons beside
  **Save to Env**. Download pretty-prints the body to `response.json` and shows a spinner while it
  works. If the body isn't JSON or YAML (plain text, HTML), a toast says why, instead of saving a
  quoted string or an `undefined` file.
- **Responses the engine cut short.** A JSON body over the engine's cap (256 KB for Send) used to
  show as unformatted raw text. It's now indented as far as it goes, then marked "… truncated".
  - **Copy** still copies the partial text, and now warns how much of the body that was (for
    example "256.0 KB of 1.2 MB"). Response sizes of 1 MB and over now show in MB.
  - The download button reads **Export partial** and saves the text that arrived as
    `response.partial.txt`, rather than refusing.
  - Bodies are indented without being re-encoded, so values show exactly as sent. For example, big
    integers are no longer rounded the way `JSON.parse` rounds them. js-yaml is now loaded only when a
  download starts, so it no longer comes with every import of `cn`. The file's temporary URL is
  released after a delay, because releasing it immediately can cancel the download in Firefox and
  Safari.
- **Caps in the UI.** The run panel shows this engine's limits (req/s, in flight, run length,
  timeout). Settings over a cap are named before you run, and **Run** stays disabled until they
  fit, instead of the engine refusing the run. Breakpoint and rate-limit runs longer than the
  duration cap say how many steps they'll get through. `GET /api/health` now includes `caps`.

- **Rate-limit discovery.** Load Test → Model → **Rate limit** steps the rate up until more than
  1% of a step's requests get 429, then stops. Results show where 429s started, the highest rate
  that went through, each step's share of 429s, and the rate-limit headers the API sent
  (`Retry-After`, `X-RateLimit-*`, `RateLimit-*`) on the first 429 and on normal responses.

- **Spike load test.** Load Test → Model → **Spike** runs a base rate, a burst at a higher rate,
  then the base rate again. Results show before / during / after (requests, achieved rate,
  p50/p99, errors) and **how long the target took to recover**: when its p99 and errors were back
  near the baseline and stayed there, or that it hadn't by the end of the run.

- **Breakpoint load test.** Load Test → Model → **Breakpoint** steps the rate up (start rate, +N%
  every step, up to a max) until a step breaks a limit: more than X% errors (drops at the
  in-flight cap count) or p99 over Y ms. The run stops at the first step that breaks. Results show
  the rate that **held**, where it **broke** and why, and every step's achieved rate, p50/p99
  (with the limit marked) and errors. The form shows the planned steps and their total length.
  Runs longer than the duration cap stop there and say so. Reports have a new `breakpoint` field.

- **Postman import.** **Import** accepts a Postman collection (v2.0 / v2.1 JSON). Folders become
  groups ("Folder / Subfolder"), collection variables become the collection's variables, `:id`
  path variables become `{{id}}`, and auth is inherited from folders and the collection as in
  Postman (Bearer, Basic, API key). Disabled headers and query rows come in switched off.
  `{{$guid}}` / `{{$randomUUID}}` become `{{uuid}}` and `{{$randomInt}}` becomes `{{int:0..1000}}`;
  other dynamic values, file bodies, unsupported auth and scripts (which aren't run) are listed to
  fix by hand. A Postman environment file gets a clear message instead of an error about specs.

- **Phase timings.** Latency and load results have a **Where the time went** section: DNS (once
  per run), connect (TCP + TLS) for the requests that opened a connection, waiting for the server,
  and download, as p50/p99 and a bar of an average request. It counts how many requests opened a
  new connection, and suggests keep-alive when connecting costs more than the server. A single
  Send shows the connect time next to TTFB when it opened a connection. Reports have a new
  `phases` field (and samples `connectMs`).

- **curl import.** Paste a curl command into a request's URL field and it fills in the method, URL,
  query, headers, auth and body (Undo is in the notice). Paste one or more commands into **Import**
  (e.g. DevTools' "Copy all as cURL") and they become a collection; when they share an origin it
  becomes the collection's `base`. Reads bash and Windows `cmd` quoting, `-d`/`--data-*`,
  `--data-urlencode`, `--json`, `-F`, `-G`, `-u`, `-b`, and `Authorization` headers (moved to the
  Auth tab). Anything that reads a file (`-d @file`, `-F f=@file`) is listed to fill in by hand,
  and credentials get a reminder that they're stored as plain text.

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

- **README rewritten** to cover what Kestrel does and how to run it: every test type (spike,
  breakpoint, rate-limit, concurrency included), every import format, getting started, Docker, the
  reference server, main settings and responsible use. It links to the hosted app at
  https://kestrel-oqjm.onrender.com. Internal details (storage layout, auth
  internals, the SQLite migration, full variable list) are gone; `.env.example` stays the full
  configuration reference.
- Endpoints in the sidebar are listed alphabetically within each group (by name, or URL when
  unnamed), the same way groups are sorted: ignoring case, with numbers in order.
- Endpoint groups in the sidebar can be collapsed: click a group's name (it shows how many
  endpoints it holds). Groups start open; filtering shows every group with a match open; adding an
  endpoint to a closed group opens it.
- Every form uses the shared `Form` component (`components/form`): sign-in, forgot and reset
  password, change password, and the inline forms for new collections, groups, environments,
  variables and secrets. `Form` gained `autoComplete`/`autoFocus` and `hideLabel` for fields,
  `trim: false` for values that must be kept exactly (passwords, templates), and `toastOnInvalid`
  for forms that already show each error inline.
- Collections in the sidebar open and close independently: clicking an open collection closes it,
  so none has to be open, and a **Collapse all** button closes them all at once. Opening a collection
  doesn't select an endpoint or change the active one; selecting or adding an endpoint inside a
  collection makes it active, so the editor always uses the right one.
  After a reload the dashboard starts blank: every collection closed, no request open and no
  breadcrumb, until you pick an endpoint. Only a collection you've just created or imported opens
  by itself, and deleting the open endpoint falls back to another open tab, never to one you
  didn't open.
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

- **The workspace page no longer crashes with "Maximum update depth exceeded".** The run panel's
  list of token requests, added with token refresh, was rebuilt as a new array on every store
  read, so React re-rendered without end. It's now derived once per change of the collections.
- **Run history now updates when a run finishes.** Before, a new run only appeared after a reload,
  because history refreshed only while a run it already knew about was in progress.
- **Workspace isolation now has two more checks behind the `search_path` pin**, plus a warning:
  - **The pin:** the engine reads back the schema each transaction will use, in the same round trip
    as setting it. A missing schema, or a pin that didn't take, stops the transaction before
    anything runs.
  - **The cached store:** before serving a request, the engine checks the store it got belongs to
    the workspace it just authorised, so a cache bug can't hand one user another's workspace.
  - **Superuser warning:** at startup, the engine warns when it connects to Postgres as a
    superuser. The warning includes the SQL for a limited `kestrel_app` role. The local Docker
    database connects as one, so you'll see it there.
- **Signing in.** Three bugs stopped it, and are fixed:
  - **Wrong route:** the sign-in page posted to `/api/auth/signin`, but the engine's route is
    `/api/auth/login`.
  - **Malformed workspace IDs:** the engine refused the sign-in request with a "UUID parsing
    failed" error. `generateUUID` builds IDs from `crypto.getRandomValues` where
    `crypto.randomUUID` is missing (plain http on a LAN address). A malformed ID already saved in
    the browser is dropped and replaced.
  - **Signed straight back out:** in `pnpm dev` the UI always called the engine at `127.0.0.1`.
    From `localhost` or a LAN address that's another site, so the browser dropped the
    `SameSite=Lax` session cookie and the next request got a 401. The UI now calls the engine on
    the page's own hostname (port from `KESTREL_PORT`), unless `KESTREL_ENGINE_URL` is set.
- **Auth errors are toasts.** Every error in sign-in, forgot and reset password, email
  verification and the security settings is a toast. On the reset page, the toast has a **Get a new
  link** button. Field errors such as "That's not your current password" stay under their field.
- **Readable error messages.** The engine's messages are shown in sentence case. When there's no
  message, you get a plain explanation instead of axios's "Request failed with status code …". A
  request the engine couldn't parse says to reload and try again, with the detail in the console.
- **The engine's API errors are always JSON** (`{"error": "…"}`).
  - An unknown `/api` path gets a JSON 404 that names the method and path, instead of the UI's HTML
    404 page.
  - Axum's own plain-text refusals (a body that isn't valid JSON, a missing field, the wrong method,
    an oversized body) are wrapped the same way, keeping their status and headers.
- The browser console no longer warns about `scroll-behavior: smooth` on every load. `<html>` now
  has `data-scroll-behavior="smooth"`, which tells Next.js to turn smooth scrolling off during route
  changes.
- **Imports over 2 MB.** `POST /api/import` used axum's 2 MB default body limit, so specs between
  2 and 10 MB were refused even though the handler, and its "larger than 10 MB" message, allow them.
  The route now takes up to 20 MB, which leaves room for the JSON escaping of the text.
- **Add endpoint** in the sidebar (and the + on a group) adds an endpoint again; it had only been
  making the collection active.
- Choosing **No environment** no longer breaks sending and runs. It used to be saved as an
  environment called "No Environment", which the engine rejected as unknown. It's now saved as no
  environment, and a workspace that already saved that name is repaired when it loads.
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
