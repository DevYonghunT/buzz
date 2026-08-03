import assert from "node:assert/strict";
import test from "node:test";

import { isCatalogAdminRequiredError } from "./catalogError.ts";

test("isCatalogAdminRequiredError: Error carrying the identifier returns true", () => {
  assert.equal(
    isCatalogAdminRequiredError(new Error("catalog-admin-required")),
    true,
  );
});

// Tauri's error channel wraps the command's `Err(String)` before it reaches
// JS, so the identifier is not guaranteed to be the whole message.
test("isCatalogAdminRequiredError: wrapped Error message returns true", () => {
  assert.equal(
    isCatalogAdminRequiredError(
      new Error("invoke failed: catalog-admin-required"),
    ),
    true,
  );
});

// Not every rejection reaching the card is an `Error` instance — the IPC layer
// can hand back a bare string.
test("isCatalogAdminRequiredError: raw string returns true", () => {
  assert.equal(isCatalogAdminRequiredError("catalog-admin-required"), true);
});

test("isCatalogAdminRequiredError: a different backend error returns false", () => {
  assert.equal(
    isCatalogAdminRequiredError(new Error("relay unreachable: 403 Forbidden")),
    false,
  );
});

// The card reads `query.error`, which is `null` until something fails. If this
// returned true the panel would show "ask an administrator" on every first
// render, before any call had been refused.
test("isCatalogAdminRequiredError: null returns false", () => {
  assert.equal(isCatalogAdminRequiredError(null), false);
});

test("isCatalogAdminRequiredError: undefined returns false", () => {
  assert.equal(isCatalogAdminRequiredError(undefined), false);
});
