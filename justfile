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

# Build the minos-daemon binary with env vars baked into the Rust compile.
# profile = release | debug
build-daemon profile='release':
    @just check-env >/dev/null
    @if [ -z "${MINOS_BACKEND_URL:-}" ]; then \
        echo "error: MINOS_BACKEND_URL must be set in .env.local for build-daemon"; \
        exit 1; \
    fi
    MINOS_BACKEND_URL="$MINOS_BACKEND_URL" \
    CF_ACCESS_CLIENT_ID="${CF_ACCESS_CLIENT_ID:-}" \
    CF_ACCESS_CLIENT_SECRET="${CF_ACCESS_CLIENT_SECRET:-}" \
    cargo build -p minos-daemon --bin minos-daemon --profile {{profile}}

# Build the mobile Rust FFI staticlib for a given target.
# target  = aarch64-apple-ios | aarch64-apple-ios-sim | x86_64-apple-ios | <android targets>
# profile = release | debug
build-mobile-rust target='aarch64-apple-ios' profile='release':
    @just check-env >/dev/null
    @if [ -z "${MINOS_BACKEND_URL:-}" ]; then \
        echo "error: MINOS_BACKEND_URL required for build-mobile-rust"; \
        exit 1; \
    fi
    MINOS_BACKEND_URL="$MINOS_BACKEND_URL" \
    CF_ACCESS_CLIENT_ID="${CF_ACCESS_CLIENT_ID:-}" \
    CF_ACCESS_CLIENT_SECRET="${CF_ACCESS_CLIENT_SECRET:-}" \
    cargo build -p minos-ffi-frb --target {{target}} --profile {{profile}}

# Build a Release iOS app via xcodebuild. Sets MINOS_BUILD_VIA_JUST=1
# so the project's Pre-Build Run Script Phase doesn't fail (added in
# Phase 5). Env vars MINOS_BACKEND_URL / CF_ACCESS_CLIENT_* are exported
# into the xcodebuild environment so cargokit's build_pod.sh inherits
# them and cargo build picks them up via option_env!.
#
# configuration = Release | Debug
build-mobile-ios configuration='Release':
    @just check-env >/dev/null
    @if [ -z "${MINOS_BACKEND_URL:-}" ]; then \
        echo "error: MINOS_BACKEND_URL required for build-mobile-ios"; \
        exit 1; \
    fi
    cd apps/mobile && flutter build ios --config-only --release
    cd apps/mobile/ios && \
    MINOS_BUILD_VIA_JUST=1 \
    MINOS_BACKEND_URL="$MINOS_BACKEND_URL" \
    CF_ACCESS_CLIENT_ID="${CF_ACCESS_CLIENT_ID:-}" \
    CF_ACCESS_CLIENT_SECRET="${CF_ACCESS_CLIENT_SECRET:-}" \
    xcodebuild \
        -workspace Runner.xcworkspace \
        -scheme Runner \
        -configuration {{configuration}} \
        -sdk iphoneos \
        -destination 'generic/platform=iOS' \
        build

# Hot-reload dev workflow. Runs `flutter run` in debug mode with --dart-define
# for Cloudflare Access, and exports MINOS_BACKEND_URL into the parent shell
# so cargokit's Rust compile (triggered by flutter's first build) picks it up.
dev-mobile-ios:
    @just check-env >/dev/null
    @if [ -z "${MINOS_BACKEND_URL:-}" ]; then \
        echo "error: MINOS_BACKEND_URL required for dev-mobile-ios"; \
        exit 1; \
    fi
    cd apps/mobile && \
    MINOS_BUILD_VIA_JUST=1 \
    MINOS_BACKEND_URL="$MINOS_BACKEND_URL" \
    CF_ACCESS_CLIENT_ID="${CF_ACCESS_CLIENT_ID:-}" \
    CF_ACCESS_CLIENT_SECRET="${CF_ACCESS_CLIENT_SECRET:-}" \
    flutter run \
        --dart-define=CF_ACCESS_CLIENT_ID="${CF_ACCESS_CLIENT_ID:-}" \
        --dart-define=CF_ACCESS_CLIENT_SECRET="${CF_ACCESS_CLIENT_SECRET:-}"

# Stub: Android APK build. No Pre-Build hook on Android yet; this recipe
# exists for parity. If Android stops being out-of-scope, harden it.
build-mobile-android:
    @just check-env >/dev/null
    cd apps/mobile && \
    MINOS_BACKEND_URL="$MINOS_BACKEND_URL" \
    CF_ACCESS_CLIENT_ID="${CF_ACCESS_CLIENT_ID:-}" \
    CF_ACCESS_CLIENT_SECRET="${CF_ACCESS_CLIENT_SECRET:-}" \
    flutter build apk \
        --dart-define=CF_ACCESS_CLIENT_ID="${CF_ACCESS_CLIENT_ID:-}" \
        --dart-define=CF_ACCESS_CLIENT_SECRET="${CF_ACCESS_CLIENT_SECRET:-}"

# Print the CF Access rotation runbook. Pure documentation; no state mutation.
rotate-cf-access:
    @cat docs/ops/secrets-rotation.md
