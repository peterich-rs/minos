import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  formatListActivityTime,
  formatLocalClock,
  formatLocalDateTime,
  formatRelative,
} from "./time.ts";

/** Fixed local noon on 2026-08-06 (device TZ applies to wall clock only). */
const NOW = new Date(2026, 7, 6, 12, 0, 0).getTime();

function atLocal(
  y: number,
  m0: number,
  d: number,
  h = 0,
  min = 0,
): number {
  return new Date(y, m0, d, h, min, 0).getTime();
}

describe("formatLocalClock", () => {
  it("returns empty for invalid ms", () => {
    assert.equal(formatLocalClock(0), "");
    assert.equal(formatLocalClock(Number.NaN), "");
  });

  it("formats local HH:mm", () => {
    const ms = atLocal(2026, 7, 6, 9, 5);
    const s = formatLocalClock(ms);
    assert.match(s, /^\d{2}:\d{2}$/);
    assert.equal(s, formatLocalClock(ms));
  });
});

describe("formatListActivityTime", () => {
  it("returns empty for invalid ms", () => {
    assert.equal(formatListActivityTime(0, NOW), "");
  });

  it("today → local clock", () => {
    const ms = atLocal(2026, 7, 6, 9, 15);
    assert.equal(formatListActivityTime(ms, NOW), formatLocalClock(ms));
  });

  it("yesterday → Yesterday", () => {
    const ms = atLocal(2026, 7, 5, 18, 0);
    assert.equal(formatListActivityTime(ms, NOW), "Yesterday");
  });

  it("within last 6 days → weekday", () => {
    const ms = atLocal(2026, 7, 3, 10, 0); // Mon if 6th is Thu
    const label = formatListActivityTime(ms, NOW);
    assert.notEqual(label, "Yesterday");
    assert.notEqual(label, formatLocalClock(ms));
    // Locale weekday short is non-empty
    assert.ok(label.length >= 2);
  });

  it("same year older → mon day", () => {
    const ms = atLocal(2026, 0, 15, 12, 0);
    const label = formatListActivityTime(ms, NOW);
    assert.match(label, /15/);
  });

  it("prior year includes year", () => {
    const ms = atLocal(2024, 11, 25, 12, 0);
    const label = formatListActivityTime(ms, NOW);
    assert.match(label, /2024/);
  });
});

describe("formatRelative", () => {
  it("uses short ages under a day", () => {
    assert.equal(formatRelative(NOW - 2_000, NOW), "now");
    assert.equal(formatRelative(NOW - 30_000, NOW), "30s");
    assert.equal(formatRelative(NOW - 5 * 60_000, NOW), "5m");
    assert.equal(formatRelative(NOW - 3 * 3600_000, NOW), "3h");
  });

  it("falls through to list buckets for multi-day ages", () => {
    const yday = atLocal(2026, 7, 5, 10, 0);
    assert.equal(formatRelative(yday, NOW), "Yesterday");
  });
});

describe("formatLocalDateTime", () => {
  it("returns empty for invalid", () => {
    assert.equal(formatLocalDateTime(0), "");
  });
});
