#!/usr/bin/env bash
# IM Reliability G2 / B7.2 delete-list gates.
# Fail on production-path anti-patterns. Docs, tests asserting absence, and
# known residuals (whitelist) are excluded via path globs / comments.
#
# Usage: from repo root
#   ./scripts/im-reliability-gates.sh
# Exit 0 = pass; non-zero = gate red.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

RED=0
pass() { echo "PASS  $1"; }
fail() { echo "FAIL  $1"; echo "      $2"; RED=1; }

# ---------------------------------------------------------------------------
# 1) client_message_id: None hardcode on send path (C1.1)
#    Production send must pass through the parameter. Spec docs naming the
#    deleted anti-pattern and this script are ignored.
# ---------------------------------------------------------------------------
matches="$(
  rg -n 'client_message_id:\s*None' \
    --glob '!**/target/**' \
    --glob '!**/node_modules/**' \
    --glob '!**/docs/**' \
    --glob '!**/*.md' \
    --glob '!scripts/**' \
    crates/ apps/ 2>/dev/null || true
)"
if [[ -z "${matches}" ]]; then
  pass "no client_message_id: None in production code"
else
  fail "client_message_id: None found" "${matches}"
fi

# ---------------------------------------------------------------------------
# 2) body soft-dedupe 120s window (production cloud-timeline / merge paths)
#    Active soft-dedupe logic is forbidden. Comments that document removal
#    ("no soft-dedupe") are allowed.
# ---------------------------------------------------------------------------
# Look for 120_000 / 120000 near soft-dedupe in cloud-timeline production file.
soft_window="$(
  rg -n '120_000|120000' apps/desktop/src/shared/lib/cloud-timeline.ts 2>/dev/null || true
)"
if [[ -n "${soft_window}" ]]; then
  fail "cloud-timeline.ts still has 120s window constant" "${soft_window}"
else
  pass "cloud-timeline.ts has no 120s soft-dedupe window"
fi

# Ensure merge docs claim id-only (sanity: soft-dedupe phrase only in "no …" comments).
# Any "soft-dedupe" / SOFT_DEDUPE mention must be documentation of removal only
# (comment lines: //, block *, or C2: markers). No identifiers as live code.
soft_impl="$(
  rg -n 'soft.?dedupe|SOFT_DEDUPE' \
    apps/desktop/src/shared/lib/cloud-timeline.ts \
    apps/desktop/src/store/workspace/live-ingress.ts \
    apps/desktop/src/features/chat/Timeline.tsx \
    2>/dev/null || true
)"
soft_bad=""
if [[ -n "${soft_impl}" ]]; then
  while IFS= read -r line; do
    [[ -z "${line}" ]] && continue
    # Comment-only: line content after file:line: starts with // or * or contains C2:
    content="${line#*:[0-9]*:}"
    # rg -n format is path:line:content
    content="$(echo "${line}" | sed -E 's/^[^:]+:[0-9]+://')"
    if echo "${content}" | rg -q '^\s*(//|/\*|\* )|C2:'; then
      continue
    fi
    soft_bad+="${line}"$'\n'
  done <<< "${soft_impl}"
fi
if [[ -z "${soft_bad// }" ]]; then
  pass "no active soft-dedupe implementation in timeline/live paths"
else
  fail "soft-dedupe-like production matches" "${soft_bad}"
fi

# ---------------------------------------------------------------------------
# 3) SessionLifecycle COUNT-only stub / fake DidWork
# ---------------------------------------------------------------------------
if rg -n 'SELECT COUNT\(\*\) FROM agent_sessions|COUNT-only stub that returns DidWork' \
  crates/minos-backend/src/jobs/stale_session_sweeper.rs 2>/dev/null \
  | rg -v 'Replaces the former COUNT-only' >/dev/null; then
  fail "stale_session_sweeper still COUNT-only" \
    "$(rg -n 'COUNT' crates/minos-backend/src/jobs/stale_session_sweeper.rs || true)"
else
  if rg -n 'end_stale_host_sessions|expire_completion_watches' \
    crates/minos-backend/src/jobs/stale_session_sweeper.rs >/dev/null; then
    pass "SessionLifecycleJob real end/expire (not COUNT-only stub)"
  else
    fail "SessionLifecycleJob missing end/expire paths" ""
  fi
fi

# ---------------------------------------------------------------------------
# 4) presence "callers should" lies (production code + architecture docs)
#    Historical work artifacts are not part of the repository.
# ---------------------------------------------------------------------------
lies="$(
  rg -n 'callers should check presence|caller should check presence' \
    --glob '!**/EVIDENCE.md' \
    --glob '!scripts/**' \
    crates/ apps/ docs/architecture-*.md 2>/dev/null || true
)"
if [[ -z "${lies}" ]]; then
  pass "no presence 'callers should' lies in code/architecture"
else
  fail "presence hand-off lies found" "${lies}"
fi

# ---------------------------------------------------------------------------
# 5) reaction event_id path must not use Uuid::new_v4()
#    Formula lives in reaction_event_id(); outbox_id UUID is unrelated.
# ---------------------------------------------------------------------------
if rg -n 'fn reaction_event_id' -A 20 crates/minos-backend/src/store/social/delivery.rs \
  | rg -n 'Uuid::new_v4' >/dev/null 2>&1; then
  fail "reaction_event_id uses Uuid::new_v4" ""
else
  if rg -n 'social-reaction-\{' crates/minos-backend/src/store/social/delivery.rs >/dev/null; then
    pass "reaction event_id formula (no Uuid::new_v4 on event_id)"
  else
    fail "reaction_event_id formula missing" ""
  fi
fi

# ---------------------------------------------------------------------------
# 6) messageSeq ?? 0 pseudo-order
# ---------------------------------------------------------------------------
pseudo="$(
  rg -n 'messageSeq\s*\?\?\s*0' \
    --glob '!**/node_modules/**' \
    --glob '!**/docs/**' \
    apps/desktop 2>/dev/null || true
)"
if [[ -z "${pseudo}" ]]; then
  pass "no messageSeq ?? 0 in desktop"
else
  fail "messageSeq ?? 0 found" "${pseudo}"
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
if [[ "${RED}" -eq 0 ]]; then
  echo "im-reliability-gates: ALL PASS"
  exit 0
fi
echo "im-reliability-gates: FAILED"
exit 1
