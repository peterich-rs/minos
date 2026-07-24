/**
 * Pure merge policy tests (duplicated helpers shape — workspace helpers use
 * `@/` imports that node:test cannot resolve without a loader).
 */
import assert from "node:assert/strict";
import { describe, it } from "node:test";
import type { TranscriptItem } from "./daemon.ts";

/**
 * Keep this file's merge helpers in sync with store/workspace/helpers.ts
 * subagent collapse (node:test cannot resolve @/ imports from helpers).
 */
function subagentDisplayKey(it: TranscriptItem): string | null {
  if (it.messageId) return it.messageId.slice(0, 12);
  if (it.requestId) return `tool:${it.requestId}`;
  const m = it.text.match(/#([A-Za-z0-9_]+)/);
  return m?.[1] ?? null;
}

function subagentCardsMatch(a: TranscriptItem, b: TranscriptItem): boolean {
  if (a.kind !== "subagent" || b.kind !== "subagent") return false;
  if (a.id === b.id) return true;
  if (a.requestId && b.requestId && a.requestId === b.requestId) return true;
  if (a.messageId && b.messageId && a.messageId === b.messageId) return true;
  if (a.messageId && b.id === `subagent:${a.messageId}`) return true;
  if (b.messageId && a.id === `subagent:${b.messageId}`) return true;
  if (a.requestId && b.id === `subagent:tool:${a.requestId}`) return true;
  if (b.requestId && a.id === `subagent:tool:${b.requestId}`) return true;
  const ka = subagentDisplayKey(a);
  const kb = subagentDisplayKey(b);
  return Boolean(ka && kb && ka === kb);
}

function mergeSubagentCard(
  cur: TranscriptItem,
  incoming: TranscriptItem,
): TranscriptItem {
  const messageId = incoming.messageId || cur.messageId || null;
  const requestId = incoming.requestId || cur.requestId || null;
  const id = messageId
    ? `subagent:${messageId}`
    : requestId
      ? `subagent:tool:${requestId}`
      : cur.id;
  const title =
    incoming.title && incoming.title !== "subagent"
      ? incoming.title
      : cur.title && cur.title !== "subagent"
        ? cur.title
        : (incoming.title ?? cur.title ?? "opencode");
  const preferIncoming =
    incoming.seq >= cur.seq ||
    (/\b(completed|failed|interrupted)\b/i.test(incoming.text) &&
      /\bRunning\b/i.test(cur.text));
  return {
    ...cur,
    id,
    kind: "subagent",
    text: preferIncoming ? incoming.text : cur.text,
    title,
    detail: incoming.detail || cur.detail,
    messageId,
    requestId,
    seq: Math.max(cur.seq, incoming.seq),
    tsMs: Math.max(cur.tsMs, incoming.tsMs),
  };
}

function collapseDuplicateSubagentCards(
  items: TranscriptItem[],
): TranscriptItem[] {
  const subIdx: number[] = [];
  items.forEach((it, i) => {
    if (it.kind === "subagent") subIdx.push(i);
  });
  if (subIdx.length <= 1) return items;
  const next = items.slice();
  const drop = new Set<number>();
  for (let a = 0; a < subIdx.length; a++) {
    for (let b = a + 1; b < subIdx.length; b++) {
      const ia = subIdx[a]!;
      const ib = subIdx[b]!;
      if (drop.has(ia) || drop.has(ib)) continue;
      if (!subagentCardsMatch(next[ia]!, next[ib]!)) continue;
      next[ia] = mergeSubagentCard(next[ia]!, next[ib]!);
      drop.add(ib);
    }
  }
  if (drop.size === 0) return items;
  return next.filter((_, i) => !drop.has(i));
}

function isTranscriptSegmentBreaker(kind: string): boolean {
  return (
    kind === "tool" ||
    kind === "tool_result" ||
    kind === "tool_error" ||
    kind === "subagent" ||
    kind === "status" ||
    kind === "approval" ||
    kind === "question" ||
    kind === "error"
  );
}

function findOpenStreamSlot(
  out: TranscriptItem[],
  kind: string,
  messageId: string,
): number {
  for (let i = out.length - 1; i >= 0; i--) {
    const it = out[i]!;
    if (it.kind !== kind || it.messageId !== messageId) continue;
    const closed = out
      .slice(i + 1)
      .some((x) => isTranscriptSegmentBreaker(x.kind));
    return closed ? -1 : i;
  }
  return -1;
}

function isToolLifecycleKind(kind: string): boolean {
  return kind === "tool" || kind === "tool_result" || kind === "tool_error";
}

function mergeToolLifecycleItems(
  cur: TranscriptItem,
  incoming: TranscriptItem,
): TranscriptItem {
  const curDone = cur.kind === "tool_result" || cur.kind === "tool_error";
  const inDone =
    incoming.kind === "tool_result" || incoming.kind === "tool_error";
  let kind = cur.kind;
  if (inDone) {
    kind =
      cur.kind === "tool_error" || incoming.kind === "tool_error"
        ? "tool_error"
        : "tool_result";
  } else if (!curDone && incoming.kind === "tool") {
    kind = "tool";
  }
  const preferIncomingDetail =
    inDone && (incoming.detail?.length ?? 0) >= (cur.detail?.length ?? 0);
  return {
    ...cur,
    ...incoming,
    id: cur.id,
    kind,
    title: incoming.title?.trim() ? incoming.title : cur.title,
    text: incoming.text?.trim() ? incoming.text : cur.text,
    detail: preferIncomingDetail
      ? (incoming.detail ?? cur.detail)
      : (cur.detail ?? incoming.detail),
    requestId: incoming.requestId || cur.requestId,
    messageId: incoming.messageId || cur.messageId,
    seq: Math.max(cur.seq, incoming.seq),
    tsMs: Math.max(cur.tsMs, incoming.tsMs),
  };
}

function mergeTranscriptItems(
  prev: TranscriptItem[],
  incoming: TranscriptItem[],
): TranscriptItem[] {
  if (incoming.length === 0) {
    return collapseDuplicateSubagentCards(prev);
  }
  const byId = new Map(prev.map((it) => [it.id, it]));
  const out = [...prev];
  for (const item of incoming) {
    if (byId.has(item.id)) {
      const idx = out.findIndex((x) => x.id === item.id);
      if (idx >= 0) {
        const cur = out[idx]!;
        // Mirror helpers.ts: live delta frames must append, not replace.
        if (
          (item.kind === "assistant" ||
            item.kind === "user" ||
            item.kind === "reasoning" ||
            item.kind === "text") &&
          item.kind === cur.kind
        ) {
          const isTail = idx === out.length - 1;
          let nextText = cur.text;
          if (item.text !== cur.text) {
            if (
              item.text.length >= cur.text.length &&
              (cur.text.length === 0 || item.text.startsWith(cur.text))
            ) {
              nextText = item.text;
            } else if (isTail) {
              nextText = cur.text + item.text;
            }
          }
          out[idx] = {
            ...cur,
            ...item,
            id: cur.id,
            text: nextText,
            seq: Math.max(cur.seq, item.seq),
            tsMs: item.tsMs || cur.tsMs,
          };
          byId.set(cur.id, out[idx]!);
          continue;
        }
        if (isToolLifecycleKind(cur.kind) && isToolLifecycleKind(item.kind)) {
          out[idx] = mergeToolLifecycleItems(cur, item);
          byId.set(cur.id, out[idx]!);
          continue;
        }
        out[idx] = item;
        byId.set(item.id, item);
      }
      continue;
    }
    if (item.kind === "subagent") {
      let idx = out.findIndex((x) => subagentCardsMatch(x, item));
      if (idx < 0 && item.messageId) {
        const orphans = out
          .map((x, i) => ({ x, i }))
          .filter(
            ({ x }) =>
              x.kind === "subagent" &&
              !x.messageId &&
              /\bRunning\b/i.test(x.text),
          );
        if (orphans.length === 1) idx = orphans[0]!.i;
      }
      if (idx >= 0) {
        out[idx] = mergeSubagentCard(out[idx]!, item);
        continue;
      }
    }
    if (
      item.messageId &&
      (item.kind === "assistant" ||
        item.kind === "user" ||
        item.kind === "reasoning")
    ) {
      const idx = findOpenStreamSlot(out, item.kind, item.messageId);
      if (idx >= 0 && out[idx]) {
        const cur = out[idx]!;
        const nextText =
          item.text.length >= cur.text.length
            ? item.text
            : cur.text + item.text;
        out[idx] = {
          ...cur,
          text: nextText,
          seq: Math.max(cur.seq, item.seq),
          tsMs: item.tsMs || cur.tsMs,
        };
        continue;
      }
    }
    out.push(item);
  }
  return collapseDuplicateSubagentCards(out);
}

function item(
  partial: Partial<TranscriptItem> & Pick<TranscriptItem, "id" | "kind" | "text">,
): TranscriptItem {
  return {
    role: null,
    tsMs: 1,
    seq: 1,
    ...partial,
  };
}

describe("mergeTranscriptItems timeline freeze", () => {
  it("does not rewrite assistant text once tools sit after it", () => {
    const prev = [
      item({
        id: "a1",
        kind: "assistant",
        text: "first",
        messageId: "msg1",
      }),
      item({ id: "t1", kind: "tool", text: "foo.ts", title: "read" }),
    ];
    const incoming = [
      item({
        id: "a-new",
        kind: "assistant",
        text: "first SHOULD NOT MERGE UP",
        messageId: "msg1",
      }),
    ];
    const out = mergeTranscriptItems(prev, incoming);
    assert.equal(out[0]!.text, "first");
    assert.equal(out.length, 3);
    assert.equal(out[2]!.text, "first SHOULD NOT MERGE UP");
  });

  it("extends open tail assistant segment", () => {
    const prev = [
      item({
        id: "a1",
        kind: "assistant",
        text: "hel",
        messageId: "msg1",
      }),
    ];
    const incoming = [
      item({
        id: "a2",
        kind: "assistant",
        text: "hello",
        messageId: "msg1",
      }),
    ];
    const out = mergeTranscriptItems(prev, incoming);
    assert.equal(out.length, 1);
    assert.equal(out[0]!.text, "hello");
  });

  it("appends live TextDelta chunks that share stable id", () => {
    // frame_to_ingest_dto builds one assembler per frame → each item is only
    // the latest token with the same stable id. Must concatenate, not replace.
    const prev = [
      item({
        id: "s1:assistant:msg1",
        kind: "assistant",
        text: "I'll",
        messageId: "msg1",
        seq: 1,
      }),
    ];
    let out = mergeTranscriptItems(prev, [
      item({
        id: "s1:assistant:msg1",
        kind: "assistant",
        text: " explore",
        messageId: "msg1",
        seq: 2,
      }),
    ]);
    assert.equal(out.length, 1);
    assert.equal(out[0]!.text, "I'll explore");
    out = mergeTranscriptItems(out, [
      item({
        id: "s1:assistant:msg1",
        kind: "assistant",
        text: " stores",
        messageId: "msg1",
        seq: 3,
      }),
    ]);
    assert.equal(out[0]!.text, "I'll explore stores");
  });

  it("prefers cumulative snapshot when live frame sends full prefix", () => {
    const prev = [
      item({
        id: "s1:assistant:msg1",
        kind: "assistant",
        text: "hel",
        messageId: "msg1",
      }),
    ];
    const out = mergeTranscriptItems(prev, [
      item({
        id: "s1:assistant:msg1",
        kind: "assistant",
        text: "hello",
        messageId: "msg1",
        seq: 2,
      }),
    ]);
    assert.equal(out[0]!.text, "hello");
  });

  it("upserts subagent by requestId without duplicating", () => {
    const prev = [
      item({
        id: "subagent:tool:call1",
        kind: "subagent",
        text: "Running subagent opencode #ses_1 · running",
        requestId: "call1",
        messageId: "ses_1",
        detail: "Explore",
      }),
    ];
    const incoming = [
      item({
        id: "subagent:ses_1",
        kind: "subagent",
        text: "Ran subagent opencode #ses_1 · completed",
        requestId: "call1",
        messageId: "ses_1",
        detail: "Explore",
      }),
    ];
    const out = mergeTranscriptItems(prev, incoming);
    assert.equal(out.length, 1);
    assert.match(out[0]!.text, /completed|Ran/);
  });

  it("collapses running + completed rows that used different ids", () => {
    const prev = [
      item({
        id: "subagent:tool:call1",
        kind: "subagent",
        text: "Running subagent opencode #ses_072f · running",
        requestId: "call1",
        seq: 1,
      }),
      item({
        id: "subagent:ses_072f",
        kind: "subagent",
        text: "Ran subagent opencode #ses_072f · completed",
        messageId: "ses_072f",
        requestId: "call1",
        detail: "Explore desktop",
        seq: 2,
      }),
      item({
        id: "subagent:orphan",
        kind: "subagent",
        text: "Ran subagent subagent #ses_072f · completed",
        messageId: "ses_072f",
        seq: 3,
      }),
    ];
    // Empty incoming still collapses legacy multi-row cards.
    const collapsed = mergeTranscriptItems(prev, []);
    assert.equal(
      collapsed.filter((x) => x.kind === "subagent").length,
      1,
      JSON.stringify(collapsed.filter((x) => x.kind === "subagent").map((s) => s.text)),
    );
    const out2 = mergeTranscriptItems(prev, [
      item({
        id: "subagent:ses_072f",
        kind: "subagent",
        text: "Ran subagent opencode #ses_072f · completed",
        messageId: "ses_072f",
        requestId: "call1",
        seq: 4,
      }),
    ]);
    const subs = out2.filter((x) => x.kind === "subagent");
    assert.equal(subs.length, 1, JSON.stringify(subs.map((s) => s.text)));
    assert.match(subs[0]!.text, /completed|Ran/);
    assert.ok(!/subagent subagent/.test(subs[0]!.text));
  });

  it("does not demote tool_result back to open tool on title refine", () => {
    const prev = [
      item({
        id: "tool:tc1",
        kind: "tool_result",
        text: "a.ts",
        title: "search_replace",
        detail: "--- a/a.ts\n+++ b/a.ts\n+x",
        requestId: "tc1",
        seq: 2,
      }),
    ];
    const out = mergeTranscriptItems(prev, [
      item({
        id: "tool:tc1",
        kind: "tool",
        text: "a.ts",
        title: "edit: a.ts",
        requestId: "tc1",
        seq: 3,
      }),
    ]);
    assert.equal(out.length, 1);
    assert.equal(out[0]!.kind, "tool_result");
    assert.equal(out[0]!.title, "edit: a.ts");
    assert.ok(out[0]!.detail?.includes("+x"));
  });

  it("promotes open tool to tool_result on progressive complete", () => {
    const prev = [
      item({
        id: "tool:tc2",
        kind: "tool",
        text: "b.ts",
        title: "search_replace",
        requestId: "tc2",
        seq: 1,
      }),
    ];
    const out = mergeTranscriptItems(prev, [
      item({
        id: "tool:tc2",
        kind: "tool_result",
        text: "b.ts",
        title: "edit: b.ts",
        detail: "+1 -0",
        requestId: "tc2",
        seq: 2,
      }),
    ]);
    assert.equal(out[0]!.kind, "tool_result");
    assert.equal(out[0]!.detail, "+1 -0");
  });
});
