# Multi-stage production image for minos-backend.
#
# VPS policy: never `docker build` on the production host (4G RAM / 60G SSD).
# Build in CI and pull from GHCR only.
#
# Toolchain must match rust-toolchain.toml / workspace rust-version (1.97).

# ── Build stage ──────────────────────────────────────────────────────────────
FROM rust:1.97-bookworm AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
        cmake \
        clang \
        git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy the workspace. `.dockerignore` strips clients, docs, and local targets.
COPY . .

# Release binary only. Features default to sqlite+postgres (see crate Cargo.toml).
ENV CARGO_TERM_COLOR=never
RUN cargo build --release -p minos-backend --bin minos-backend \
    && strip target/release/minos-backend

# ── Runtime stage ────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system minos \
    && useradd --system --gid minos --home-dir /app --shell /usr/sbin/nologin minos

WORKDIR /app

COPY --from=builder /build/target/release/minos-backend /app/minos-backend
# Migrations are compiled into the binary via sqlx::migrate!; kept on disk for ops inspection.
COPY --from=builder /build/crates/minos-backend/migrations /app/migrations

RUN mkdir -p /data && chown -R minos:minos /app /data

USER minos

ENV MINOS_BACKEND_LISTEN=0.0.0.0:8787
ENV RUST_LOG=info

EXPOSE 8787

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8787/health/live >/dev/null || exit 1

ENTRYPOINT ["/app/minos-backend"]
