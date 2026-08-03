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
 * Returns true when `error` is the catalog commands' "you are not a community
 * administrator" refusal.
 *
 * Accepts both `Error` instances and raw strings, matching
 * {@link import("@/shared/lib/relayError").isRelayUnreachableError} — callers
 * pass whatever the Tauri IPC layer hands them without pre-normalizing, and
 * that is not always an `Error`.
 *
 * Matches by substring rather than equality because the identifier travels
 * through Tauri's error channel and React Query, either of which may wrap it.
 */
export function isCatalogAdminRequiredError(error: unknown): boolean {
  if (error instanceof Error) {
    return error.message.includes(CATALOG_ADMIN_REQUIRED);
  }
  if (typeof error === "string") {
    return error.includes(CATALOG_ADMIN_REQUIRED);
  }
  return false;
}
