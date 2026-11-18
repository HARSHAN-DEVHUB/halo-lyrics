import { test } from "node:test";
import assert from "node:assert/strict";
import { gradientFromPalette } from "./color.ts";

const HEX = /^#[0-9A-F]{6}$/;

test("gradient: four valid colours from a palette", () => {
  const g = gradientFromPalette(["#E0405A", "#3060C0", "#F0C020"]);
  assert.equal(g.length, 4);
  for (const c of g) assert.match(c, HEX);
});

test("gradient: base is darker than the blobs", () => {
  const lum = (hex: string) => parseInt(hex.slice(1, 3), 16) + parseInt(hex.slice(3, 5), 16) + parseInt(hex.slice(5, 7), 16);
  const [base, a, b, c] = gradientFromPalette(["#E0405A", "#3060C0", "#F0C020"]);
  for (const blob of [a, b, c]) assert.ok(lum(base) < lum(blob), `${base} should be darker than ${blob}`);
});

test("gradient: no palette or junk falls back", () => {
  assert.equal(gradientFromPalette(undefined).length, 4);
  assert.deepEqual(gradientFromPalette([]), gradientFromPalette(["nope"]));
});

test("gradient: a one-colour palette still fills every slot", () => {
  const g = gradientFromPalette(["#3060C0"]);
  for (const c of g) assert.match(c, HEX);
});
