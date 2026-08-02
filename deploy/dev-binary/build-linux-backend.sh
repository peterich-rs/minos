#!/usr/bin/env bash
# Build a linux/amd64 minos-backend release binary for VPS binary-bypass deploys.
#
# Default method is a one-shot Docker image build that only extracts the binary
# (works on macOS hosts; does not leave a running stack). Alternative methods:
# cargo zigbuild / cross when those toolchains are installed.
#
# Usage:
#   ./deploy/dev-binary/build-linux-backend.sh [--method docker|cross|zig|native] [--out PATH]
#   ./deploy/dev-binary/build-linux-backend.sh --help
#
# Output default: dist/minos-backend-linux-amd64
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
METHOD="${MINOS_BUILD_METHOD:-docker}"
OUT="${MINOS_BUILD_OUT:-$ROOT/dist/minos-backend-linux-amd64}"
TARGET="x86_64-unknown-linux-gnu"
IMAGE_TAG="${MINOS_BUILD_IMAGE_TAG:-minos-backend:dev-binary-local}"

usage() {
  cat <<'EOF'
Build linux x86_64 minos-backend release binary (extract-only; no VPS source build).

Usage:
  build-linux-backend.sh [options]

Options:
  --method METHOD   docker (default) | cross | zig | native
  --out PATH        Output binary path (default: dist/minos-backend-linux-amd64)
  --image-tag TAG   Docker image tag when --method=docker (default: minos-backend:dev-binary-local)
  -h, --help        Show this help

Environment:
  MINOS_BUILD_METHOD, MINOS_BUILD_OUT, MINOS_BUILD_IMAGE_TAG  same as flags

Methods:
  docker   Multi-stage Dockerfile build; copy /app/minos-backend out (recommended on macOS)
  cross    cross-rs: cargo cross build --release --target x86_64-unknown-linux-gnu -p minos-backend
  zig      cargo zigbuild --release --target x86_64-unknown-linux-gnu -p minos-backend
  native   cargo build --release --target x86_64-unknown-linux-gnu (Linux host only)

Required tools by method:
  docker   Docker Engine (linux/amd64 platform emulation on Apple Silicon via buildx/qemu)
  cross    cargo-cross + Docker
  zig      cargo-zigbuild + Zig
  native   Rust toolchain with target x86_64-unknown-linux-gnu + linker
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --method)
      METHOD="${2:?--method requires a value}"
      shift 2
      ;;
    --out)
      OUT="${2:?--out requires a path}"
      shift 2
      ;;
    --image-tag)
      IMAGE_TAG="${2:?--image-tag requires a value}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

mkdir -p "$(dirname "$OUT")"
cd "$ROOT"

case "$METHOD" in
  docker)
    if ! command -v docker >/dev/null 2>&1; then
      echo "error: docker not found; install Docker or use --method cross|zig|native" >&2
      exit 1
    fi
    echo "==> docker build (linux/amd64) → $IMAGE_TAG"
    # Force amd64 so Apple Silicon hosts produce a VPS-compatible binary.
    docker build --platform linux/amd64 -t "$IMAGE_TAG" -f Dockerfile .
    cid="$(docker create --platform linux/amd64 "$IMAGE_TAG")"
    cleanup() { docker rm -f "$cid" >/dev/null 2>&1 || true; }
    trap cleanup EXIT
    docker cp "$cid:/app/minos-backend" "$OUT"
    trap - EXIT
    cleanup
    ;;
  cross)
    if ! command -v cross >/dev/null 2>&1; then
      echo "error: cross not found (cargo install cross --git https://github.com/cross-rs/cross)" >&2
      exit 1
    fi
    echo "==> cross build --target $TARGET -p minos-backend"
    cross build --release --target "$TARGET" -p minos-backend --bin minos-backend
    cp -f "$ROOT/target/$TARGET/release/minos-backend" "$OUT"
    if command -v strip >/dev/null 2>&1; then
      strip "$OUT" 2>/dev/null || true
    fi
    ;;
  zig)
    if ! command -v cargo-zigbuild >/dev/null 2>&1 && ! cargo zigbuild -h >/dev/null 2>&1; then
      echo "error: cargo-zigbuild not found (cargo install cargo-zigbuild; install Zig)" >&2
      exit 1
    fi
    echo "==> cargo zigbuild --target $TARGET -p minos-backend"
    cargo zigbuild --release --target "$TARGET" -p minos-backend --bin minos-backend
    cp -f "$ROOT/target/$TARGET/release/minos-backend" "$OUT"
    if command -v strip >/dev/null 2>&1; then
      strip "$OUT" 2>/dev/null || true
    fi
    ;;
  native)
    if [[ "$(uname -s)" != "Linux" ]]; then
      echo "error: --method native requires a Linux host (use docker on macOS)" >&2
      exit 1
    fi
    echo "==> cargo build --release --target $TARGET -p minos-backend"
    rustup target add "$TARGET" >/dev/null 2>&1 || true
    cargo build --release --target "$TARGET" -p minos-backend --bin minos-backend
    cp -f "$ROOT/target/$TARGET/release/minos-backend" "$OUT"
    strip "$OUT" 2>/dev/null || true
    ;;
  *)
    echo "error: unknown --method '$METHOD' (docker|cross|zig|native)" >&2
    exit 2
    ;;
esac

chmod +x "$OUT"
echo "==> built: $OUT"
if command -v file >/dev/null 2>&1; then
  file "$OUT" || true
fi
ls -lh "$OUT"
echo "SHA256: $(shasum -a 256 "$OUT" | awk '{print $1}')"
