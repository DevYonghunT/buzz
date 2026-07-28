/**
 * Product identity for the relay-served web client.
 *
 * This is the third copy of the same product layer (the others are
 * `desktop/src-tauri/src/product.rs` and `desktop/src/shared/product/index.ts`)
 * because the three builds share no module graph. They must agree: this client
 * *generates* the deep links the desktop app *registers*, so a scheme that
 * drifts here produces invite and connect buttons that open nothing.
 *
 * As in the other copies, protocol identifiers (`buzz:nostr-identity` and
 * friends) are shared vocabulary with relays and other clients — they are not
 * product strings and are not renamed here.
 */

/** URL scheme the desktop app registers with the OS. */
export const DEEP_LINK_SCHEME = "schoolx";

/** `schoolx://` — prefix for building deep links. */
export const DEEP_LINK_URL_PREFIX = `${DEEP_LINK_SCHEME}://` as const;

/** Product name shown in this client's copy ("Open in …", "… desktop app"). */
export const PRODUCT_NAME = "SchoolX";
