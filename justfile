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
        if [ -z "${MINOS_BACKEND_URL:-}" ] && [ -z "${MINOS_JWT_SECRET:-}" ]; then \
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

# Internal: patch the built macOS app Info.plist with config loaded by just.
# Xcode calls this after ProcessInfoPlistFile so Finder/Xcode launches get the

# same runtime RelayConfig as command-line builds without checking secrets in.
_patch-macos-info-plist plist_path:
    @just check-env >/dev/null
    @if [ ! -f "{{ plist_path }}" ]; then \
        echo "error: Info.plist not found at {{ plist_path }}"; \
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

# Backend-focused verification for formal-cutover work.
check-backend:
    cargo xtask gen-backend-platform-contract --check
    cargo test -p minos-backend

# Regenerate the backend runtime contract, OpenAPI, and websocket schema artifacts.
gen-backend-platform-contract:
    cargo xtask gen-backend-platform-contract

# Run the standalone web admin client.
dev-web:
    cd apps/web && pnpm dev

# Production build for the standalone web admin client.
build-web:
    cd apps/web && pnpm build

# Build and serve the production web bundle locally.
preview-web host='0.0.0.0' port='5173':
    cd apps/web && pnpm build && pnpm preview --host "{{ host }}" --port "{{ port }}"

# Web-only verification.
check-web:
    cd apps/web && pnpm check

# Host desktop shell (Tauri + React). Starts Vite on :1420, or reuses it if already up.
dev-desktop:
    cd apps/desktop && pnpm tauri:dev

# Frontend-only desktop UI in the browser (mock data; same Vite port :1420).
# Safe to run first, then `just dev-desktop` — Tauri will reuse this server.
dev-desktop-ui:
    cd apps/desktop && pnpm dev

# Production Tauri bundle for the host desktop shell.
build-desktop:
    cd apps/desktop && pnpm tauri:build

# Desktop frontend typecheck.
check-desktop:
    cd apps/desktop && pnpm check

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
    cargo build -p minos-ffi-frb --target {{ target }} --profile {{ profile }}

# Build a Release iOS app via xcodebuild. MINOS_BACKEND_URL is exported
# into the xcodebuild environment; Cargokit
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
    xcodebuild \
        -workspace Runner.xcworkspace \
        -scheme Runner \
        -configuration {{ configuration }} \
        -sdk iphoneos \
        -destination 'generic/platform=iOS' \
        build

# Hot-reload dev workflow. Cargokit still self-bootstraps through just
# before the Rust compile.
dev-mobile-ios:
    @just check-env >/dev/null
    @if [ -z "${MINOS_BACKEND_URL:-}" ]; then \
        echo "error: MINOS_BACKEND_URL required for dev-mobile-ios"; \
        exit 1; \
    fi
    cd apps/mobile && \
    MINOS_BACKEND_URL="$MINOS_BACKEND_URL" \
    flutter run

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
    flutter run -d android

# Build Android APK with just-loaded env passthrough.
build-mobile-android:
    @just check-env >/dev/null
    @if [ -z "${MINOS_BACKEND_URL:-}" ]; then \
        echo "error: MINOS_BACKEND_URL required for build-mobile-android"; \
        exit 1; \
    fi
    cd apps/mobile && \
    MINOS_BACKEND_URL="$MINOS_BACKEND_URL" \
    flutter build apk
