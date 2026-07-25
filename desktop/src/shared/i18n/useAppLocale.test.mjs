import assert from "node:assert/strict";
import test from "node:test";

import { i18n } from "./index.ts";
import { setAppLocale } from "./useAppLocale.ts";

test("setAppLocale still changes the session language when localStorage access is blocked", async () => {
  const originalWindowDescriptor = Object.getOwnPropertyDescriptor(
    globalThis,
    "window",
  );
  const blockedWindow = {};
  Object.defineProperty(blockedWindow, "localStorage", {
    get() {
      throw new DOMException("blocked", "SecurityError");
    },
  });
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: blockedWindow,
  });

  try {
    await i18n.changeLanguage("en");
    await assert.doesNotReject(() => setAppLocale("ko"));
    assert.equal(i18n.resolvedLanguage, "ko");
  } finally {
    if (originalWindowDescriptor) {
      Object.defineProperty(globalThis, "window", originalWindowDescriptor);
    } else {
      delete globalThis.window;
    }
  }
});
