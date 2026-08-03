import assert from "node:assert/strict";
import test from "node:test";

import {
  isCatalogAdminRequiredError,
  isCatalogGateRefusalError,
  isCatalogMembershipUnavailableError,
} from "./catalogError.ts";

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

test("isCatalogMembershipUnavailableError: matches its own identifier", () => {
  assert.equal(
    isCatalogMembershipUnavailableError(
      new Error("catalog-membership-unavailable"),
    ),
    true,
  );
  assert.equal(
    isCatalogMembershipUnavailableError("catalog-membership-unavailable"),
    true,
  );
  assert.equal(isCatalogMembershipUnavailableError(null), false);
});

// The card picks one branch by testing these in order, so an error matching
// both would render whichever comes first and silently hide the other. Neither
// identifier is a substring of the other; this pins that, because the card's
// copy is only correct while they stay disjoint.
test("the two refusals never match the same error", () => {
  const adminRequired = new Error("catalog-admin-required");
  const membershipUnavailable = new Error("catalog-membership-unavailable");

  assert.equal(isCatalogMembershipUnavailableError(adminRequired), false);
  assert.equal(isCatalogAdminRequiredError(membershipUnavailable), false);
});

// Drives the preflight query's `retry` predicate. A false negative here costs
// a wasted round-trip; a false positive would stop retrying a genuine relay
// blip, which the global `retry: 1` exists to ride out.
test("isCatalogGateRefusalError: covers both refusals and nothing else", () => {
  assert.equal(
    isCatalogGateRefusalError(new Error("catalog-admin-required")),
    true,
  );
  assert.equal(
    isCatalogGateRefusalError(new Error("catalog-membership-unavailable")),
    true,
  );
  assert.equal(
    isCatalogGateRefusalError(new Error("relay unreachable: 502")),
    false,
  );
  assert.equal(isCatalogGateRefusalError(null), false);
});
