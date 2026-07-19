# Multi-stage Dockerfile for minos-backend.
#
# Build stage: compiles the Rust binary with musl for a static binary.
# Final stage: minimal alpine image with non-root user.

# ── Build stage ──────────────────────────────────────────────────────
FROM rust:1.82-alpine AS builder

RUN apk add --no-cache musl-dev openssl-dev pkgconfig

WORKDIR /build
COPY . .

# Build the backend binary in release mode.
RUN cargo build --release -p minos-backend --bin minos-backend

# ── Final stage ──────────────────────────────────────────────────────
FROM alpine:3.20

RUN apk add --no-cache ca-certificates

# Create non-root user.
RUN addgroup -S minos && adduser -S minos -G minos

WORKDIR /app

# Copy the binary from the builder stage.
COPY --from=builder /build/target/release/minos-backend /app/minos-backend

# Copy migrations for SQLite (if used).
COPY --from=builder /build/crates/minos-backend/migrations /app/migrations

# Create data directory for SQLite.
RUN mkdir -p /data && chown minos:minos /data

# Switch to non-root user.
USER minos

# Default environment variables.
ENV MINOS_BACKEND_LISTEN=0.0.0.0:8787
ENV MINOS_BACKEND_DB=/data/minos-backend.db
ENV RUST_LOG=info

EXPOSE 8787

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD wget -qO- http://localhost:8787/health/live || exit 1

ENTRYPOINT ["/app/minos-backend"]
