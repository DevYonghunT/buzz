import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_APP_LOCALE,
  getLocaleStorageFromHost,
  LOCALE_STORAGE_KEY,
  normalizeAppLocale,
  persistAppLocale,
  readPreferredSystemLocales,
  readStoredAppLocale,
  resolveInitialAppLocale,
} from "./locale.ts";

test("normalizeAppLocale accepts supported regional locale variants", () => {
  assert.equal(normalizeAppLocale("ko-KR"), "ko");
  assert.equal(normalizeAppLocale("EN_us"), "en");
  assert.equal(normalizeAppLocale("ko-Hang-KR"), "ko");
  assert.equal(normalizeAppLocale("ja-JP"), null);
  assert.equal(normalizeAppLocale("en--US"), null);
  assert.equal(normalizeAppLocale(null), null);
});

test("resolveInitialAppLocale prefers a stored locale", () => {
  assert.equal(resolveInitialAppLocale("en", ["ko-KR"]), "en");
});

test("resolveInitialAppLocale uses the first supported system locale without a saved choice", () => {
  assert.equal(resolveInitialAppLocale(null, ["en-US", "ko-KR"]), "en");
  assert.equal(resolveInitialAppLocale(null, ["ja-JP", "ko-KR"]), "ko");
});

test("resolveInitialAppLocale uses the Korean product default when no supported locale exists", () => {
  assert.equal(resolveInitialAppLocale(null), DEFAULT_APP_LOCALE);
  assert.equal(
    resolveInitialAppLocale("ja-JP", ["zh-Hant-TW"]),
    DEFAULT_APP_LOCALE,
  );
});

test("readPreferredSystemLocales keeps browser preference order and tolerates blocked metadata", () => {
  assert.deepEqual(
    readPreferredSystemLocales({
      language: "en-US",
      languages: ["ko-KR", "en-US"],
    }),
    ["ko-KR", "en-US"],
  );

  const blockedNavigator = {};
  Object.defineProperties(blockedNavigator, {
    language: {
      get() {
        throw new Error("blocked");
      },
    },
    languages: {
      get() {
        throw new Error("blocked");
      },
    },
  });
  assert.deepEqual(readPreferredSystemLocales(blockedNavigator), []);
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

test("getLocaleStorageFromHost tolerates a blocked localStorage getter", () => {
  const blockedHost = {};
  Object.defineProperty(blockedHost, "localStorage", {
    get() {
      throw new Error("blocked");
    },
  });

  assert.equal(getLocaleStorageFromHost(blockedHost), null);
});
