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
        if [ -z "${MINOS_BACKEND_URL:-}" ] && [ -z "${MINOS_JWT_SECRET:-}" ] && [ -z "${CF_ACCESS_CLIENT_ID:-}" ] && [ -z "${CF_ACCESS_CLIENT_SECRET:-}" ]; then \
            echo "error: .env.local not found and no Minos env vars are set in the parent process."; \
            echo "error: run: cp .env.example .env.local"; \
            exit 1; \
        fi; \
        echo "env source: parent process (.env.local not found)"; \
    else \
        echo "env source: .env.local"; \
    fi
    @echo "MINOS_BACKEND_URL = ${MINOS_BACKEND_URL:-<unset>}"
    @echo "MINOS_JWT_SECRET  = ${MINOS_JWT_SECRET:+<set, ${#MINOS_JWT_SECRET} chars>}"
    @echo "CF_ACCESS_CLIENT_ID     = ${CF_ACCESS_CLIENT_ID:-<unset>}"
    @echo "CF_ACCESS_CLIENT_SECRET = ${CF_ACCESS_CLIENT_SECRET:+<set>}"

# Internal: patch the built macOS app Info.plist with config loaded by just.
# Xcode calls this after ProcessInfoPlistFile so Finder/Xcode launches get the

# same runtime RelayConfig as command-line builds without checking secrets in.
_patch-macos-info-plist plist_path:
    @just check-env >/dev/null
    @if [ ! -f "{{ plist_path }}" ]; then \
        echo "error: Info.plist not found at {{ plist_path }}"; \
        exit 1; \
    fi
    @if { [ -n "${CF_ACCESS_CLIENT_ID:-}" ] && [ -z "${CF_ACCESS_CLIENT_SECRET:-}" ]; } || \
        { [ -z "${CF_ACCESS_CLIENT_ID:-}" ] && [ -n "${CF_ACCESS_CLIENT_SECRET:-}" ]; }; then \
        echo "error: CF_ACCESS_CLIENT_ID and CF_ACCESS_CLIENT_SECRET must be set together"; \
        exit 1; \
    fi
    @plist="{{ plist_path }}"; \
    set_string() { \
        key="$1"; value="$2"; \
        /usr/libexec/PlistBuddy -c "Delete :$key" "$plist" >/dev/null 2>&1 || true; \
        if [ -n "$value" ]; then \
            /usr/libexec/PlistBuddy -c "Add :$key string $value" "$plist"; \
        fi; \
    }; \
    set_string MINOS_BACKEND_URL "${MINOS_BACKEND_URL:-}"; \
    set_string CF_ACCESS_CLIENT_ID "${CF_ACCESS_CLIENT_ID:-}"; \
    set_string CF_ACCESS_CLIENT_SECRET "${CF_ACCESS_CLIENT_SECRET:-}"; \
    echo "Patched Minos runtime env into $plist"

# Run minos-backend with .env.local; requires MINOS_JWT_SECRET (32+ bytes).
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
        {{ kind }} --backend "$MINOS_BACKEND_URL"

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
    cargo build -p minos-daemon --bin minos-daemon --profile {{ profile }}

# Build the macOS app through Xcode. The generated project also calls back
# into just from its build phases, so Xcode IDE Run uses the same env path.

# configuration = Debug | Release
build-macos configuration='Debug':
    @just check-env >/dev/null
    cargo xtask gen-uniffi
    cargo xtask gen-xcode
    cd apps/macos && xcodebuild \
        -project Minos.xcodeproj \
        -scheme Minos \
        -configuration {{ configuration }} \
        -destination 'platform=macOS' \
        build

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
    cargo build -p minos-ffi-frb --target {{ target }} --profile {{ profile }}

# Build a Release iOS app via xcodebuild. Env vars MINOS_BACKEND_URL /
# CF_ACCESS_CLIENT_* are exported into the xcodebuild environment; Cargokit
# also self-bootstraps through just so direct Xcode/Flutter builds load the
# same .env.local before cargo evaluates option_env!.
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
    MINOS_BACKEND_URL="$MINOS_BACKEND_URL" \
    CF_ACCESS_CLIENT_ID="${CF_ACCESS_CLIENT_ID:-}" \
    CF_ACCESS_CLIENT_SECRET="${CF_ACCESS_CLIENT_SECRET:-}" \
    xcodebuild \
        -workspace Runner.xcworkspace \
        -scheme Runner \
        -configuration {{ configuration }} \
        -sdk iphoneos \
        -destination 'generic/platform=iOS' \
        build

# Hot-reload dev workflow. Runs `flutter run` in debug mode with --dart-define
# for Cloudflare Access, and exports MINOS_BACKEND_URL into the parent shell.

# Cargokit still self-bootstraps through just before the Rust compile.
dev-mobile-ios:
    @just check-env >/dev/null
    @if [ -z "${MINOS_BACKEND_URL:-}" ]; then \
        echo "error: MINOS_BACKEND_URL required for dev-mobile-ios"; \
        exit 1; \
    fi
    cd apps/mobile && \
    MINOS_BACKEND_URL="$MINOS_BACKEND_URL" \
    CF_ACCESS_CLIENT_ID="${CF_ACCESS_CLIENT_ID:-}" \
    CF_ACCESS_CLIENT_SECRET="${CF_ACCESS_CLIENT_SECRET:-}" \
    flutter run \
        --dart-define=CF_ACCESS_CLIENT_ID="${CF_ACCESS_CLIENT_ID:-}" \
        --dart-define=CF_ACCESS_CLIENT_SECRET="${CF_ACCESS_CLIENT_SECRET:-}"

# Hot-reload Android workflow. Mirrors `dev-mobile-ios` so Android debug runs
# stay on the same `.env.local` / cargokit path as release builds.
dev-mobile-android:
    @just check-env >/dev/null
    @if [ -z "${MINOS_BACKEND_URL:-}" ]; then \
        echo "error: MINOS_BACKEND_URL required for dev-mobile-android"; \
        exit 1; \
    fi
    cd apps/mobile && \
    MINOS_BACKEND_URL="$MINOS_BACKEND_URL" \
    CF_ACCESS_CLIENT_ID="${CF_ACCESS_CLIENT_ID:-}" \
    CF_ACCESS_CLIENT_SECRET="${CF_ACCESS_CLIENT_SECRET:-}" \
    flutter run \
        -d android \
        --dart-define=CF_ACCESS_CLIENT_ID="${CF_ACCESS_CLIENT_ID:-}" \
        --dart-define=CF_ACCESS_CLIENT_SECRET="${CF_ACCESS_CLIENT_SECRET:-}"

# Build Android APK with just-loaded env passthrough.
build-mobile-android:
    @just check-env >/dev/null
    @if [ -z "${MINOS_BACKEND_URL:-}" ]; then \
        echo "error: MINOS_BACKEND_URL required for build-mobile-android"; \
        exit 1; \
    fi
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
