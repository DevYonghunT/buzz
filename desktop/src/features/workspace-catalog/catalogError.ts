/**
 * Classifies the one error identifier the catalog commands return that the UI
 * must translate rather than display.
 *
 * The Rust backend (`desktop/src-tauri/src/commands/workspace_catalog.rs`)
 * returns this exact string from `require_community_admin` when the caller is
 * not a community owner/admin. It is an **identifier, not a sentence**: the
 * backend deliberately does not localize, because a hardcoded Korean or
 * English message there leaks to users on the other locale. The copy lives in
 * `catalog.adminRequired` (`shared/i18n/locales/{en,ko}.ts`).
 *
 * Contract: `workspace_catalog.rs` MUST keep emitting exactly this string for
 * the not-an-administrator condition. Do not change it on either side alone —
 * a mismatch degrades silently, showing the raw identifier to the user instead
 * of the explanation.
 */
const CATALOG_ADMIN_REQUIRED = "catalog-admin-required";

/**
 * Returned when the relay does not advertise NIP-43 at all, so there is no
 * community-role concept to check against.
 *
 * Deliberately distinct from {@link CATALOG_ADMIN_REQUIRED} because the two
 * ask the user for different things. "Ask an administrator" is actionable;
 * on a relay with no roster there *is* no administrator to ask, and telling
 * the community owner to go find one is a dead end. `require_relay_membership`
 * defaults to false on the relay, so this is the state of a stock dev relay.
 */
const CATALOG_MEMBERSHIP_UNAVAILABLE = "catalog-membership-unavailable";

/**
 * Returns true when `error` is the catalog commands' "you are not a community
 * administrator" refusal.
 *
 * Accepts both `Error` instances and raw strings, matching
 * {@link import("@/shared/lib/relayError").isRelayUnreachableError} — callers
 * pass whatever the Tauri IPC layer hands them without pre-normalizing, and
 * that is not always an `Error`.
 *
 * Matches by substring rather than equality defensively. Today nothing wraps
 * the value: `invokeTauri` (`shared/api/tauri.ts`) rejects with the command's
 * bare `Err(String)` and React Query stores that `Error` unmodified, so the
 * message is exactly the identifier. Substring keeps the match working if a
 * future layer prefixes it, which is the failure this would otherwise hit
 * silently.
 */
export function isCatalogAdminRequiredError(error: unknown): boolean {
  return carries(error, CATALOG_ADMIN_REQUIRED);
}

/**
 * Returns true when `error` is the catalog commands' "this relay has no
 * community roles" refusal. See {@link CATALOG_MEMBERSHIP_UNAVAILABLE}.
 */
export function isCatalogMembershipUnavailableError(error: unknown): boolean {
  return carries(error, CATALOG_MEMBERSHIP_UNAVAILABLE);
}

/**
 * True for either gate refusal.
 *
 * Both are deterministic verdicts about who the caller is, so retrying cannot
 * change them — this is what lets the preflight query opt out of the global
 * `retry: 1` and paint the explanation immediately instead of after a second
 * doomed round-trip.
 */
export function isCatalogGateRefusalError(error: unknown): boolean {
  return (
    isCatalogAdminRequiredError(error) ||
    isCatalogMembershipUnavailableError(error)
  );
}

function carries(error: unknown, identifier: string): boolean {
  if (error instanceof Error) {
    return error.message.includes(identifier);
  }
  if (typeof error === "string") {
    return error.includes(identifier);
  }
  return false;
}
