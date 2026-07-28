/**
 * Product identity for the SchoolX frontend — the TypeScript half of the
 * layer whose Rust half is `src-tauri/src/product.rs`.
 *
 * Three kinds of `buzz`-ish strings live in this codebase and only the first
 * is a product string:
 *
 * 1. **Product strings** — the deep-link scheme, the bundle identifier, the
 *    app name. They name *this build*. Rebranding changes them, and each is a
 *    coexistence boundary with a co-installed Buzz. They belong here.
 *
 * 2. **Protocol identifiers** — `buzz:nostr-identity` (the identity-binding
 *    audience), `buzz-nostr-identity` (its protocol string), Nostr event
 *    kinds, relay wire values. These are shared vocabulary with relays and
 *    other clients; renaming one breaks interop with every peer that did not
 *    rename in lockstep. They stay `buzz`-prefixed and do **not** belong here.
 *
 * 3. **Internal namespacing** — the `buzz:` prefix on localStorage keys
 *    (`buzz:text-scale`) and DOM custom-event names (`buzz:open-create-agent`).
 *    These never leave this webview, and localStorage is already partitioned
 *    per bundle identifier, so they are not a coexistence boundary at all.
 *    Renaming them would silently reset every stored user preference for no
 *    isolation gain, so they are deliberately left alone.
 *
 * The display name is **not** here either: it is a translated string
 * (`app.productName`) so Korean users see 스쿨엑스 while the OS-level bundle
 * name stays ASCII `SchoolX`.
 */

/** URL scheme this build generates deep links with, without the colon. */
export const DEEP_LINK_SCHEME = "schoolx";

/** `schoolx:` — the form `URL.protocol` returns. */
export const DEEP_LINK_PROTOCOL = `${DEEP_LINK_SCHEME}:` as const;

/** `schoolx://` — for prefix checks against raw hrefs. */
export const DEEP_LINK_URL_PREFIX = `${DEEP_LINK_SCHEME}://` as const;

/**
 * Scheme SchoolX inherited links from. Read-only: links written before the
 * rename — and links written by Buzz users in a shared community — still say
 * `buzz://`, and those messages are SchoolX's own history.
 *
 * The app therefore *reads* this scheme and never *writes* it. It is also not
 * registered with the OS (see `product.rs`), so this only ever matches link
 * text already inside the app, never an OS-routed launch.
 */
export const LEGACY_DEEP_LINK_SCHEME = "buzz";

/** `buzz:` — the form `URL.protocol` returns for legacy links. */
export const LEGACY_DEEP_LINK_PROTOCOL = `${LEGACY_DEEP_LINK_SCHEME}:` as const;

/** `buzz://` — for prefix checks against raw legacy hrefs. */
export const LEGACY_DEEP_LINK_URL_PREFIX =
  `${LEGACY_DEEP_LINK_SCHEME}://` as const;
