# syntax=docker/dockerfile:1
#
# Kestrel in one image: the Rust engine with the Next.js UI embedded, serving both on one port.
#   docker build -t kestrel .
#   docker run --rm -p 127.0.0.1:7070:7070 -e KESTREL_DATABASE_URL=… -e KESTREL_SECRETS_KEY=… kestrel
# Data lives in Postgres (or use `docker compose up -d --build`, which brings its own).
# There is no login yet: publish the port on 127.0.0.1 (as above) or a private network only.

# ── UI: static export into /app/kestrel/out ─────────────────────────────────────────────────────
FROM node:24-slim AS ui
ENV NEXT_TELEMETRY_DISABLED=1
RUN npm install -g pnpm@10.18.1
WORKDIR /app/kestrel
COPY kestrel/package.json kestrel/pnpm-lock.yaml kestrel/pnpm-workspace.yaml ./
RUN pnpm install --frozen-lockfile
COPY kestrel/ ./
# No engine URL or token baked in: the engine serves this build on its own origin and injects the
# session token into the page (engine/src/api/ui.rs).
ENV KESTREL_ENGINE_URL="" KESTREL_TOKEN=""
RUN pnpm build

# ── Engine: release build with the UI embedded ──────────────────────────────────────────────────
FROM rust:1-bookworm AS engine
# aws-lc-sys (rustls' crypto) may need CMake depending on the target.
RUN apt-get update && apt-get install -y --no-install-recommends cmake && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY engine ./engine
# rust-embed reads ../kestrel/out relative to the engine crate at compile time.
COPY --from=ui /app/kestrel/out ./kestrel/out
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked -p engine \
    && cp target/release/engine /usr/local/bin/kestrel-engine

# ── Runtime ─────────────────────────────────────────────────────────────────────────────────────
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --home /data kestrel \
    && mkdir -p /data
COPY --from=engine /usr/local/bin/kestrel-engine /usr/local/bin/kestrel-engine

# `::` so Fly's private (IPv6) network can reach it. Still no public exposure unless you publish it.
ENV KESTREL_BIND=:: \
    KESTREL_PORT=7070 \
    KESTREL_WORKSPACE_DIR=/data \
    RUST_LOG=engine=info
WORKDIR /data
VOLUME /data
EXPOSE 7070

# Volumes (Fly's included) mount root-owned: take ownership, then drop to the unprivileged user.
ENTRYPOINT ["sh", "-c", "chown -R kestrel:kestrel /data && exec setpriv --reuid=kestrel --regid=kestrel --init-groups kestrel-engine"]
