# 0012 · SQLite via sqlx from Day One

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-23 |
| Deciders | fannnzhang |

## Context

The relay (ADR 0009) maintains three classes of state:

1. **Devices** — one row per ever-seen device, with its role, display name, and hashed secret.
2. **Pairings** — one row per established pair, undirected (stored as `(device_a, device_b)` with `a < b`).
3. **Pairing tokens** — short-lived one-shot tokens issued by `request_pairing_token`, consumed by `pair`, GC'd on expiry.

Early discussion considered an in-memory-first approach (everything in `HashMap` / `DashMap`) with SQLite added later. Two considerations pushed toward persistence on day one:

- **Restart cost.** In-memory state means every relay restart (e.g., `cargo run` during development, LaunchDaemon reload in prod) invalidates every pairing. The user re-scans a QR every time. For a development loop that iterates on the relay multiple times a day, this becomes painful fast, and it rewards shortcut workarounds that would need to be unwound when persistence lands.
- **Migration shape is easy when the schema is small.** Writing the three tables now is half a day's work. Retrofitting persistence across a codebase that grew around in-memory assumptions is harder, because every access pattern becomes a potential migration point.

## Decision

Persist relay state in SQLite from the first commit of `minos-relay`, accessed through **`sqlx`** with the `sqlite` and `runtime-tokio` features, and migrated via **`sqlx::migrate!`** against plain SQL files under `crates/minos-relay/migrations/`.

- Tables defined in §8 of `minos-relay-backend-design.md`; each is a separate numbered migration (`0001_devices.sql`, `0002_pairings.sql`, `0003_pairing_tokens.sql`).
- All tables use `STRICT` mode and explicit integer epoch timestamps (`INTEGER NOT NULL`); no `TEXT`-dated timestamps, no type affinity loopholes.
- `DeviceSecret` is stored as an `argon2id` hash, never in plaintext. The plaintext value is transmitted to the client exactly once, inside the `Paired` event.
- Connection pool: `SqlitePool` with a conservative cap (defaults are fine for MVP load).
- CI runs `cargo sqlx prepare --check` to ensure offline query metadata is current; `sqlx-data.json` is committed.
- DB file location: `./minos-relay.db` by default in dev; `~/Library/Application Support/minos-relay/db.sqlite` on prod Mac. Overridable via `MINOS_RELAY_DB` env / `--db` flag.

## Consequences

**Positive**
- Relay restarts do not invalidate pairings. Development loop is tolerable; production recovery after a crash reuses the same trust state.
- SQLite is a single file, backs up trivially (copy the file), and is inspectable with `sqlite3` / any SQLite tool. Zero operator overhead.
- `sqlx`'s compile-time query checking catches SQL-level schema drift at build time rather than in production. Offline mode (`sqlx-data.json`) keeps CI unblocked without a live database.
- Async-native: queries run on tokio without `spawn_blocking` hops. Fits the rest of the stack (tokio, axum).
- Migrations are plain SQL files, numbered. No DSL, no migration-tool-specific syntax. Reading `git log migrations/` tells the full schema history.

**Neutral / accepted cost**
- `sqlx` has a non-trivial build time (procedural macros + query-file scanning). A clean workspace build is measurably slower than `rusqlite`. On incremental builds the cost is absorbed. Accepted.
- CI needs a `cargo sqlx prepare` step before `cargo build` in offline mode, and `sqlx-data.json` becomes a committed artifact that must stay current (diffs on schema PRs). Documented in the ops notes.
- SQLite concurrent write throughput caps out below a server-grade DB. For MVP message rates (a handful of device handshakes / day) this is several orders of magnitude beyond any plausible demand.

**Negative / explicit trade-off**
- SQLite migrations are forward-only. Rolling back a schema change in production means writing a compensating migration, not reverting. Accepted — forward-only is the norm for production databases and the discipline is easier than the fully-reversible alternative.
- Scaling beyond single-node eventually forces a migration to Postgres (or similar). The `sqlx` abstraction keeps most of the query code portable; the hot spots are schema and migrations. Deferred until the product reaches a scale where it matters, which is well beyond MVP.

## Alternatives Rejected

### In-memory first, SQLite later

Keep all state in `DashMap` / `HashMap`, migrate to SQLite when pain justifies it. Rejected:
- Development friction of re-scanning QRs on every restart was the first thing we'd work around, probably with ad-hoc file dumps that pretend to be persistence. That's the worst of both worlds.
- Retrofitting persistence across access patterns grown around in-memory assumptions is strictly harder than writing it upfront. The current schema is three small tables.

### `rusqlite` instead of `sqlx`

`rusqlite` is synchronous, has faster build times, and a smaller dependency footprint. Rejected:
- Sync API forces `spawn_blocking` at every call site to avoid blocking the tokio runtime. The call-site ergonomics degrade the more call sites exist.
- No compile-time query checking. Schema drift lands in production, not in CI.
- No built-in migration runner comparable to `sqlx::migrate!`. Hand-rolling a runner is work we do not need to do.
- Build-time cost of `sqlx` is real but amortizes well on the incremental builds that dominate developer time.

### `diesel` (sync) or `sea-orm` (async)

ORMs with typed models and richer query DSLs. Rejected:
- Schema for MVP is three tables, ten columns total. An ORM is overkill at this scale; the abstraction cost dwarfs the benefit.
- Both ORMs add a second opinionated layer on top of the SQL we would otherwise write in 200 characters. Debugging becomes reading ORM-generated SQL instead of our own.
- `sqlx`'s query-macro approach gives typed results from plain SQL strings, which is the right trade-off at this table count.

### Document database (RocksDB / sled)

Considered for the session registry specifically. Rejected:
- Session registry is ephemeral (in-memory, cleared on restart) by design — no persistence need for it.
- For devices / pairings / tokens, relational constraints (FK cascades on `forget_peer`, undirected-pair uniqueness check) are natural in SQL and awkward in a KV store.
