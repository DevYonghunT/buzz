import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_APP_LOCALE,
  LOCALE_STORAGE_KEY,
  normalizeAppLocale,
  persistAppLocale,
  readStoredAppLocale,
  resolveInitialAppLocale,
} from "./locale.ts";

test("normalizeAppLocale accepts supported regional locale variants", () => {
  assert.equal(normalizeAppLocale("ko-KR"), "ko");
  assert.equal(normalizeAppLocale("EN_us"), "en");
  assert.equal(normalizeAppLocale("ja-JP"), null);
  assert.equal(normalizeAppLocale(null), null);
});

test("resolveInitialAppLocale prefers a stored locale", () => {
  assert.equal(resolveInitialAppLocale("en"), "en");
});

test("resolveInitialAppLocale uses the Korean product default without a saved choice", () => {
  assert.equal(resolveInitialAppLocale(null), DEFAULT_APP_LOCALE);
  assert.equal(resolveInitialAppLocale("ja-JP"), DEFAULT_APP_LOCALE);
});

test("locale storage helpers persist supported values and tolerate storage failures", () => {
  const values = new Map();
  const storage = {
    getItem(key) {
      return values.get(key) ?? null;
    },
    setItem(key, value) {
      values.set(key, value);
    },
  };

  persistAppLocale("ko", storage);
  assert.equal(values.get(LOCALE_STORAGE_KEY), "ko");
  assert.equal(readStoredAppLocale(storage), "ko");

  const blockedStorage = {
    getItem() {
      throw new Error("blocked");
    },
    setItem() {
      throw new Error("blocked");
    },
  };
  assert.doesNotThrow(() => persistAppLocale("en", blockedStorage));
  assert.equal(readStoredAppLocale(blockedStorage), null);
});
