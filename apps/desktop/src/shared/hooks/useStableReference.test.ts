import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { createElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import {
  useStableArrayShallow,
  useStableMap,
  useStableSet,
} from "./useStableReference.ts";

/**
 * Drive a hook once and capture its return value via SSR render.
 * Multi-render identity is covered by the pure algorithm sims below —
 * react-dom/server does not preserve hooks across trees.
 */
function probeHook<T>(useHook: () => T): T {
  let value: T | undefined;
  function Probe() {
    value = useHook();
    return null as unknown as ReactNode;
  }
  renderToStaticMarkup(createElement(Probe));
  assert.ok(value !== undefined, "hook did not produce a value");
  return value as T;
}

/** Mirrors useStableMap body with an external ref (test multi-step identity). */
function stableMapStep<K, V>(
  ref: { current: Map<K, V> },
  next: Map<K, V>,
): Map<K, V> {
  const prev = ref.current;
  if (prev !== next) {
    let equal = prev.size === next.size;
    if (equal) {
      for (const [key, value] of prev) {
        if (!next.has(key) || !Object.is(next.get(key), value)) {
          equal = false;
          break;
        }
      }
    }
    if (equal) return prev;
  }
  ref.current = next;
  return next;
}

function stableArrayStep<T extends readonly unknown[]>(
  ref: { current: T },
  next: T,
): T {
  const prev = ref.current;
  if (prev !== next) {
    let equal = prev.length === next.length;
    if (equal) {
      for (let i = 0; i < prev.length; i += 1) {
        if (!Object.is(prev[i], next[i])) {
          equal = false;
          break;
        }
      }
    }
    if (equal) return prev;
  }
  ref.current = next;
  return next;
}

function stableSetStep<T>(
  ref: { current: ReadonlySet<T> },
  next: ReadonlySet<T>,
): ReadonlySet<T> {
  const prev = ref.current;
  if (prev !== next) {
    let equal = prev.size === next.size;
    if (equal) {
      for (const value of prev) {
        if (!next.has(value)) {
          equal = false;
          break;
        }
      }
    }
    if (equal) return prev;
  }
  ref.current = next;
  return next;
}

describe("useStableMap", () => {
  it("returns the initial map on first mount", () => {
    const map = new Map([["k", 1]]);
    assert.equal(
      probeHook(() => useStableMap(map)),
      map,
    );
  });

  it("preserves reference when entries match; replaces when a value changes", () => {
    const first = new Map([["a", 1]]);
    const equal = new Map([["a", 1]]);
    const changed = new Map([["a", 2]]);
    const ref = { current: first };
    assert.equal(stableMapStep(ref, first), first);
    assert.equal(stableMapStep(ref, equal), first);
    assert.equal(stableMapStep(ref, changed), changed);
  });
});

describe("useStableArrayShallow", () => {
  it("returns the initial array on first mount", () => {
    const arr = [1, 2, 3] as const;
    assert.equal(
      probeHook(() => useStableArrayShallow(arr)),
      arr,
    );
  });

  it("preserves reference when elements are Object.is-equal", () => {
    const a = [{ id: 1 }, { id: 2 }];
    const b = [a[0]!, a[1]!];
    const c = [{ id: 1 }, { id: 2 }];
    const ref = { current: a as readonly { id: number }[] };
    assert.equal(stableArrayStep(ref, a), a);
    assert.equal(stableArrayStep(ref, b), a);
    assert.equal(stableArrayStep(ref, c), c);
  });
});

describe("useStableSet", () => {
  it("returns the initial set on first mount", () => {
    const set = new Set([1, 2]);
    assert.equal(
      probeHook(() => useStableSet(set)),
      set,
    );
  });

  it("preserves reference when membership matches", () => {
    const a = new Set(["x", "y"]);
    const b = new Set(["y", "x"]);
    const c = new Set(["x"]);
    const ref = { current: a as ReadonlySet<string> };
    assert.equal(stableSetStep(ref, a), a);
    assert.equal(stableSetStep(ref, b), a);
    assert.equal(stableSetStep(ref, c), c);
  });
});
