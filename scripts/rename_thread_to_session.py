#!/usr/bin/env python3
"""Minos-owned thread_* → session_* rename.

Skips Codex upstream protocol (minos-codex-protocol) where wire methods
remain thread/start etc.

Does NOT touch provider_session_id (CLI provider id, distinct concept).
"""

from __future__ import annotations

import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

SKIP_DIR_NAMES = {
    ".git",
    "target",
    "node_modules",
    "dist",
    "build",
    ".dart_tool",
    "Pods",
    "DerivedData",
    "__pycache__",
    ".venv",
}

# Entire crates / paths left alone (upstream Codex wire vocabulary).
SKIP_PATH_PREFIXES = (
    "crates/minos-codex-protocol/",
    "third_party/",
    "schemas/v2/",
    "schemas/codex_",
)

# File suffixes we rewrite.
TEXT_SUFFIXES = {
    ".rs",
    ".ts",
    ".tsx",
    ".js",
    ".jsx",
    ".dart",
    ".md",
    ".sql",
    ".toml",
    ".json",
    ".yml",
    ".yaml",
    ".txt",
    ".sh",
}

# Ordered replacements: longer / more specific first.
# Tuple: (pattern, replacement) — plain strings unless is_regex.
REPLACEMENTS: list[tuple[str, str, bool]] = [
    # parent / source / affected first
    ("parent_thread_id", "parent_session_id", False),
    ("parent_thread", "parent_session", False),
    ("source_thread_id", "source_session_id", False),
    ("source_thread", "source_session", False),
    ("affected_thread_ids", "affected_session_ids", False),
    ("affected_thread_id", "affected_session_id", False),
    ("parentThreadId", "parentSessionId", False),
    ("sourceThreadId", "sourceSessionId", False),
    ("affectedThreadIds", "affectedSessionIds", False),
    ("threadShortId", "sessionShortId", False),
    ("thread_short_id", "session_short_id", False),
    ("short_thread_id", "short_session_id", False),
    ("shortThreadId", "shortSessionId", False),
    # RPC / type names (snake + Pascal)
    ("InterruptThreadRequest", "InterruptSessionRequest", False),
    ("CloseThreadRequest", "CloseSessionRequest", False),
    ("ResumeThreadRequest", "ResumeSessionRequest", False),
    ("GetThreadParams", "GetSessionParams", False),
    ("ReadThreadParams", "ReadSessionParams", False),
    ("ReadThreadResponse", "ReadSessionResponse", False),
    ("ReadThreadRawHistory", "ReadSessionRawHistory", False),
    ("read_thread_raw_history", "read_session_raw_history", False),
    ("LocalThreadSnapshot", "LocalSessionSnapshot", False),
    ("ThreadSummary", "SessionSummary", False),
    ("ThreadRow", "SessionRow", False),
    ("ThreadAdded", "SessionAdded", False),
    ("ThreadStateChanged", "SessionStateChanged", False),
    ("ThreadClosed", "SessionClosed", False),
    ("ThreadNotFound", "SessionNotFound", False),
    ("threadNotFound", "sessionNotFound", False),
    ("ListThreads", "ListSessions", False),
    ("list_local_threads", "list_local_sessions", False),
    ("list_threads", "list_sessions", False),
    ("interrupt_thread", "interrupt_session", False),
    ("close_thread", "close_session", False),
    ("resume_thread", "resume_session", False),
    ("read_thread", "read_session", False),
    ("get_thread", "get_session", False),
    ("insert_thread", "insert_session", False),
    ("delete_thread", "delete_session", False),
    ("update_thread", "update_session", False),
    ("mark_thread", "mark_session", False),
    ("start_agent_with_thread_id", "start_agent_with_session_id", False),
    ("start_codex_agent_with_thread_id", "start_codex_agent_with_session_id", False),
    ("with_thread_id", "with_session_id", False),
    ("thread_provider_session_id", "session_provider_session_id", False),
    # SQL / table / path identifiers
    ("project_threads", "project_sessions", False),
    ("idx_project_threads", "idx_project_sessions", False),
    ("idx_threads_", "idx_sessions_", False),
    ("threads_by_", "sessions_by_", False),
    ("chat_messages_by_thread", "chat_messages_by_session", False),
    # Desktop / TS store keys
    ("transcriptsByThread", "transcriptsBySession", False),
    ("transcriptStatusByThread", "transcriptStatusBySession", False),
    ("transcriptHistoryByThread", "transcriptHistoryBySession", False),
    ("ByThread", "BySession", False),
    ("resumeThread", "resumeSession", False),
    ("ResumeThread", "ResumeSession", False),
    ("threadClosed", "sessionClosed", False),
    ("threadAdded", "sessionAdded", False),
    ("threadStateChanged", "sessionStateChanged", False),
    # Core id fields last (after parent_thread_id etc.)
    ("thread_id", "session_id", False),
    ("threadId", "sessionId", False),
    # Table name: careful — only whole-word-ish SQL contexts via string replace
    # of common patterns. Do AFTER thread_id so we don't mangle things twice.
    ("INTO threads", "INTO sessions", False),
    ("FROM threads", "FROM sessions", False),
    ("JOIN threads", "JOIN sessions", False),
    ("TABLE threads", "TABLE sessions", False),
    ("TABLE IF NOT EXISTS threads", "TABLE IF NOT EXISTS sessions", False),
    ("UPDATE threads", "UPDATE sessions", False),
    ("DELETE FROM threads", "DELETE FROM sessions", False),
    ("REFERENCES threads", "REFERENCES sessions", False),
    ("ON threads", "ON sessions", False),
    ('"threads"', '"sessions"', False),
    ("'threads'", "'sessions'", False),
    ("join(\"threads\")", "join(\"sessions\")", False),
    ("join('threads')", "join('sessions')", False),
    ("/threads/", "/sessions/", False),
    # Remaining bare `threads` as table in CREATE / comments — word boundary
    (r"\bthreads\b", "sessions", True),
    # Method names still containing Thread as Minos types
    ("ThreadDto", "SessionDto", False),
    ("thread_dir", "session_dir", False),
    ("delete_thread_artifacts", "delete_session_artifacts", False),
    ("list_project_threads", "list_project_sessions", False),
    ("projectThreads", "projectSessions", False),
    # Docs / prose common phrases (after identifier renames)
    ("agent thread", "agent session", False),
    ("Agent thread", "Agent session", False),
    ("persisted thread", "persisted session", False),
    ("Persisted thread", "Persisted session", False),
    ("subagent thread", "subagent session", False),
    ("Subagent thread", "Subagent session", False),
    ("orphan threads", "orphan sessions", False),
    ("top-level thread", "top-level session", False),
    ("top-level threads", "top-level sessions", False),
    ("local thread", "local session", False),
    ("Local thread", "Local session", False),
    ("the thread ", "the session ", False),
    ("a thread ", "a session ", False),
    ("this thread", "this session", False),
    ("that thread", "that session", False),
    ("per thread", "per session", False),
    ("per-thread", "per-session", False),
    ("same thread", "same session", False),
    ("each thread", "each session", False),
    ("one thread", "one session", False),
    ("active thread", "active session", False),
    ("running thread", "running session", False),
    ("closed thread", "closed session", False),
    ("spawned thread", "spawned session", False),
    ("freshly-spawned thread", "freshly-spawned session", False),
    ("thread lifecycle", "session lifecycle", False),
    ("thread events", "session events", False),
    ("thread event", "session event", False),
    ("thread status", "session status", False),
    ("thread state", "session state", False),
    ("thread list", "session list", False),
    ("Thread list", "Session list", False),
    ("thread transcript", "session transcript", False),
    ("thread history", "session history", False),
    ("thread row", "session row", False),
    ("thread rows", "session rows", False),
    ("thread key", "session key", False),
    ("thread id", "session id", False),
    ("Thread id", "Session id", False),
    ("Thread ID", "Session ID", False),
    ("thread ID", "session ID", False),
    ("`thread`", "`session`", False),
]


def should_skip(path: Path) -> bool:
    rel = path.relative_to(ROOT).as_posix()
    for prefix in SKIP_PATH_PREFIXES:
        if rel.startswith(prefix):
            return True
    parts = path.parts
    for p in parts:
        if p in SKIP_DIR_NAMES:
            return True
    if path.suffix not in TEXT_SUFFIXES and path.name not in {
        "AGENTS.md",
        "Cargo.toml",
        "Dockerfile",
    }:
        # still allow extensionless md-like at root handled above
        if path.suffix == "":
            return True
        return True
    return False


def transform(content: str) -> str:
    out = content
    for old, new, is_re in REPLACEMENTS:
        if is_re:
            out = re.sub(old, new, out)
        else:
            out = out.replace(old, new)
    return out


def rename_paths(renames: list[tuple[Path, Path]]) -> None:
    # deepest paths first so parent renames don't break children
    for src, dst in sorted(renames, key=lambda x: len(str(x[0])), reverse=True):
        if src.exists():
            dst.parent.mkdir(parents=True, exist_ok=True)
            src.rename(dst)
            print(f"rename path: {src.relative_to(ROOT)} -> {dst.relative_to(ROOT)}")


def main() -> int:
    changed_files = 0
    path_renames: list[tuple[Path, Path]] = []

    for dirpath, dirnames, filenames in os.walk(ROOT):
        # prune skip dirs in-place
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIR_NAMES]
        rel_dir = Path(dirpath).relative_to(ROOT).as_posix()
        if any(rel_dir.startswith(p.rstrip("/")) or rel_dir + "/" == p for p in SKIP_PATH_PREFIXES):
            dirnames.clear()
            continue
        if rel_dir.startswith("crates/minos-codex-protocol"):
            dirnames.clear()
            continue

        for name in filenames:
            path = Path(dirpath) / name
            if should_skip(path):
                continue
            try:
                raw = path.read_bytes()
            except OSError:
                continue
            # skip binary
            if b"\0" in raw[:8000]:
                continue
            try:
                text = raw.decode("utf-8")
            except UnicodeDecodeError:
                continue

            new_text = transform(text)
            if new_text != text:
                path.write_text(new_text, encoding="utf-8")
                changed_files += 1
                print(f"rewrite: {path.relative_to(ROOT)}")

            # file name renames
            new_name = transform(name)
            if new_name != name:
                path_renames.append((path, path.with_name(new_name)))

    rename_paths(path_renames)
    print(f"\nDone. Rewrote {changed_files} files, {len(path_renames)} path renames.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
