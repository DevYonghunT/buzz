import assert from "node:assert/strict";
import test from "node:test";

import { en } from "./locales/en.ts";
import { ko } from "./locales/ko.ts";

function collectLeafKeys(value, prefix = "") {
  return Object.entries(value).flatMap(([key, child]) => {
    const path = prefix ? `${prefix}.${key}` : key;
    return typeof child === "string" ? [path] : collectLeafKeys(child, path);
  });
}

function collectLeafValues(value) {
  return Object.values(value).flatMap((child) =>
    typeof child === "string" ? [child] : collectLeafValues(child),
  );
}

test("English and Korean translation catalogs expose the same keys", () => {
  assert.deepEqual(collectLeafKeys(ko).sort(), collectLeafKeys(en).sort());
});

test("translation catalogs do not contain blank values", () => {
  assert.equal(
    collectLeafValues(en).every((value) => value.trim().length > 0),
    true,
  );
  assert.equal(
    collectLeafValues(ko).every((value) => value.trim().length > 0),
    true,
  );
});
