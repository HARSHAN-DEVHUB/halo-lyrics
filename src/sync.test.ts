// Run with: npm test   (uses Node's built-in test runner + type stripping)
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  buildDisplayItems,
  EASE_MS,
  formatTime,
  indexAt,
  PlaybackClock,
  SEEK_THRESHOLD_MS,
} from "./sync.ts";
import { tintFromPalette } from "./color.ts";

function makeClock(start = 1_000_000) {
  let now = start;
  const clock = new PlaybackClock(() => now);
  return {
    clock,
    advance: (ms: number) => (now += ms),
    get now() {
      return now;
    },
  };
}

test("clock extrapolates while playing and freezes while paused", () => {
  const c = makeClock();
  assert.equal(c.clock.update({ positionMs: 10_000, positionAt: c.now, playing: true }), "init");
  c.advance(1500);
  assert.equal(c.clock.now(), 11_500);
  c.clock.update({ positionMs: 11_500, positionAt: c.now, playing: false });
  c.advance(5000);
  assert.equal(c.clock.now(), 11_500);
});

test("sample taken in the past is projected to now", () => {
  const c = makeClock();
  c.clock.update({ positionMs: 5000, positionAt: c.now - 250, playing: true });
  assert.equal(c.clock.now(), 5250);
});

test("a disagreement above the threshold is a seek (jump)", () => {
  const c = makeClock();
  c.clock.update({ positionMs: 10_000, positionAt: c.now, playing: true });
  c.advance(1000);
  const r = c.clock.update({ positionMs: 60_000, positionAt: c.now, playing: true });
  assert.equal(r, "seek");
  assert.equal(c.clock.now(), 60_000); // jumped immediately
});

test("small drift is eased over EASE_MS instead of jumping", () => {
  const c = makeClock();
  c.clock.update({ positionMs: 10_000, positionAt: c.now, playing: true });
  c.advance(1000);
  // Backend says we're 200 ms ahead of our prediction (11 000).
  const drift = 200;
  assert.ok(drift < SEEK_THRESHOLD_MS);
  const r = c.clock.update({ positionMs: 11_000 + drift, positionAt: c.now, playing: true });
  assert.equal(r, "ease");
  // Right now we still show the predicted value: no visible jump.
  assert.equal(c.clock.now(), 11_000);
  // Half-way through the ease we're between prediction and truth, moving forward.
  c.advance(EASE_MS / 2);
  const mid = c.clock.now();
  assert.ok(mid > 11_000 + EASE_MS / 2 && mid < 11_000 + drift + EASE_MS / 2, `mid=${mid}`);
  // After the ease we match the backend exactly.
  c.advance(EASE_MS);
  assert.equal(c.clock.now(), 11_000 + drift + EASE_MS / 2 + EASE_MS);
});

test("easing never runs time backwards for small negative drift", () => {
  const c = makeClock();
  c.clock.update({ positionMs: 10_000, positionAt: c.now, playing: true });
  c.advance(1000);
  c.clock.update({ positionMs: 10_800, positionAt: c.now, playing: true }); // 200 ms behind
  let last = c.clock.now();
  for (let i = 0; i < 20; i++) {
    c.advance(16);
    const v = c.clock.now();
    assert.ok(v >= last - 1e-9, `went backwards: ${last} -> ${v}`);
    last = v;
  }
});

test("reset makes the next sample an init", () => {
  const c = makeClock();
  c.clock.update({ positionMs: 1000, positionAt: c.now, playing: true });
  c.clock.reset();
  assert.equal(c.clock.update({ positionMs: 0, positionAt: c.now, playing: true }), "init");
  assert.equal(c.clock.now(), 0);
});

test("indexAt finds the last item that has started", () => {
  const items = [{ startMs: 0 }, { startMs: 1000 }, { startMs: 2000 }];
  assert.equal(indexAt(items, -1), -1);
  assert.equal(indexAt(items, 0), 0);
  assert.equal(indexAt(items, 1999), 1);
  assert.equal(indexAt(items, 5000), 2);
  assert.equal(indexAt([], 5), -1);
});

test("display items add intro, break and outro gaps of 4 s or more", () => {
  const lines = [
    { startMs: 5000, endMs: 9000, text: "one" },
    { startMs: 9000, endMs: 10_000, text: "♪" }, // short breather stays a line
    { startMs: 10_000, endMs: 18_000, text: "two" }, // capped at +8 s
    { startMs: 25_000, endMs: 30_000, text: "" }, // long breather becomes a gap
    { startMs: 30_000, endMs: 33_000, text: "three" },
  ];
  const items = buildDisplayItems(lines, 40_000);
  assert.deepEqual(
    items.map((i) => [i.kind, i.startMs, i.endMs]),
    [
      ["gap", 0, 5000],
      ["line", 5000, 9000],
      ["line", 9000, 10_000],
      ["line", 10_000, 18_000],
      ["gap", 18_000, 30_000],
      ["line", 30_000, 33_000],
      ["gap", 33_000, 40_000],
    ],
  );
});

test("short intro produces no gap", () => {
  const items = buildDisplayItems([{ startMs: 1500, endMs: 3000, text: "hi" }], 0);
  assert.equal(items.length, 1);
  assert.equal(items[0].kind, "line");
});

test("formatTime", () => {
  assert.equal(formatTime(0), "0:00");
  assert.equal(formatTime(61_500), "1:01");
  assert.equal(formatTime(3_725_000), "1:02:05");
});

test("tint: greyscale palettes fall back to warm off-white", () => {
  assert.equal(tintFromPalette(["#808080", "#202020", "#FFFFFF"]), "#F4F1EC");
  assert.equal(tintFromPalette(undefined), "#F4F1EC");
});

test("tint: vivid palettes are lightened but keep their hue", () => {
  const tint = tintFromPalette(["#1030C0", "#222222"]);
  assert.notEqual(tint, "#F4F1EC");
  const n = parseInt(tint.slice(1), 16);
  const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  assert.ok(b > r && b > g, `expected a blue tint, got ${tint}`);
  assert.ok(r + g + b > 450, `expected a light tint, got ${tint}`);
});
