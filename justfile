# Minos task runner. Run `just` to list recipes.
#
# Loads .env.local from the workspace root and exports every defined var
# to recipe subprocesses. CI sets vars in the parent environment instead;
# this works the same way (just doesn't care where the vars came from).
#
# Reference: docs/superpowers/specs/unified-config-pipeline-design.md §4.2
set dotenv-load := true
set dotenv-filename := ".env.local"
set dotenv-required := false
set positional-arguments := true
set shell := ["bash", "-cu"]

# Default recipe: list available commands.
default:
    @just --list

# Verify .env.local exists and has the required keys.
# Prints a summary; doesn't print secret values.
check-env:
    @if [ ! -f .env.local ]; then \
        echo "error: .env.local not found. Run: cp .env.example .env.local"; \
        exit 1; \
    fi
    @echo "MINOS_BACKEND_URL = ${MINOS_BACKEND_URL:-<unset>}"
    @echo "MINOS_JWT_SECRET  = ${MINOS_JWT_SECRET:+<set, ${#MINOS_JWT_SECRET} chars>}"
    @echo "CF_ACCESS_CLIENT_ID     = ${CF_ACCESS_CLIENT_ID:-<unset>}"
    @echo "CF_ACCESS_CLIENT_SECRET = ${CF_ACCESS_CLIENT_SECRET:+<set>}"

# Run minos-backend with values loaded from .env.local.
# Fails fast if MINOS_JWT_SECRET is unset (Config::validate enforces
# presence + ≥32 bytes at startup).
backend:
    @just check-env >/dev/null
    @if [ -z "${MINOS_JWT_SECRET:-}" ]; then \
        echo "error: MINOS_JWT_SECRET is required by minos-backend"; \
        exit 1; \
    fi
    cargo run -p minos-backend -- \
        --listen "${MINOS_BACKEND_LISTEN:-127.0.0.1:8787}" \
        --db "${MINOS_BACKEND_DB:-./minos-backend.db}"

# Workspace-wide compile + test gate. Wraps cargo xtask check-all.
check:
    cargo xtask check-all

# Run the fake-peer smoke binary with a subcommand (default: register).
# Usage: just smoke-fake-peer [register|smoke-session|pair]
smoke-fake-peer kind='register':
    @just check-env >/dev/null
    cargo run -p minos-mobile --bin fake-peer --features cli -- \
        {{kind}} --backend "$MINOS_BACKEND_URL"

# Remove all build artifacts (cargo target/ + flutter build/).
clean:
    cargo clean
    cd apps/mobile && flutter clean
