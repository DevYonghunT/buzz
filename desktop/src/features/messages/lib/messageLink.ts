/**
 * `schoolx://message` link encoding for "Copy link" / deep-link-to-message.
 *
 * Format: `schoolx://message?channel=<uuid>&id=<eventId>[&thread=<rootId>]`
 *
 * ## Legacy `buzz://message` links
 *
 * Links written before the SchoolX rename — and links written by a Buzz user
 * in a shared community — carry the `buzz:` scheme with an otherwise identical
 * shape. Those messages are SchoolX's own history, so this module **reads**
 * both schemes and **writes** only the current one.
 *
 * Reading a legacy link is not an isolation hole: it happens entirely inside
 * this webview against text the app already fetched, with no OS routing
 * involved. The coexistence boundary is which scheme the *OS* routes to this
 * build, and SchoolX registers only `schoolx` (see `src-tauri/src/product.rs`).
 * Rejecting legacy link text instead would rot old messages into dead strings
 * and buy no separation from a co-installed Buzz.
 */

// Explicit relative path + `.ts` extension: this module is imported both by
// the Vite build (which resolves `@/`) and by `messageLink.test.mjs` running
// under `node --test --experimental-strip-types`, which does not.
import {
  DEEP_LINK_PROTOCOL,
  DEEP_LINK_URL_PREFIX,
  LEGACY_DEEP_LINK_PROTOCOL,
  LEGACY_DEEP_LINK_URL_PREFIX,
} from "../../../shared/product/index.ts";

const MESSAGE_LINK_SCHEME = DEEP_LINK_PROTOCOL;
const MESSAGE_LINK_HOST = "message";

/** Schemes accepted when parsing. Only `MESSAGE_LINK_SCHEME` is ever written. */
const READABLE_MESSAGE_LINK_SCHEMES: readonly string[] = [
  DEEP_LINK_PROTOCOL,
  LEGACY_DEEP_LINK_PROTOCOL,
];

const READABLE_MESSAGE_LINK_PREFIXES: readonly string[] = [
  DEEP_LINK_URL_PREFIX,
  LEGACY_DEEP_LINK_URL_PREFIX,
];

export type MessageLinkInput = {
  channelId: string;
  messageId: string;
  /**
   * Optional thread root event id. Present when the linked message is a
   * reply (so the caller can route into a thread / forum post view).
   *
   * Currently emitted into the URL but not consumed by the click handler
   * or deep-link listener — both route via `goChannel(channelId,
   * { messageId })` and let `useAnchoredScroll` resolve the target.
   * Reserved for future "open in thread view" routing.
   */
  threadRootId?: string | null;
};

export type ParsedMessageLink = {
  channelId: string;
  messageId: string;
  threadRootId: string | null;
};

export type MessageLinkParseResult =
  | { ok: true; value: ParsedMessageLink }
  | { ok: false; reason: string };

/**
 * Build a `schoolx://message` URL for a given channel + message.
 *
 * Empty `threadRootId` is treated as "no thread" so callers can pass through
 * the result of `getThreadReference(tags).rootId` without extra null checks.
 */
export function buildMessageLink(input: MessageLinkInput): string {
  if (!input.channelId) {
    throw new Error("buildMessageLink: channelId is required");
  }
  if (!input.messageId) {
    throw new Error("buildMessageLink: messageId is required");
  }

  const params = new URLSearchParams();
  params.set("channel", input.channelId);
  params.set("id", input.messageId);
  if (input.threadRootId) {
    params.set("thread", input.threadRootId);
  }
  return `${MESSAGE_LINK_SCHEME}//${MESSAGE_LINK_HOST}?${params.toString()}`;
}

/**
 * Parse a `schoolx://message?…` URL — or a legacy `buzz://message?…` one, see
 * the module comment. Returns a discriminated result so callers can render a
 * fallback (e.g. a plain link) without throwing.
 */
export function parseMessageLink(url: string): MessageLinkParseResult {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return { ok: false, reason: "invalid-url" };
  }

  if (!READABLE_MESSAGE_LINK_SCHEMES.includes(parsed.protocol)) {
    return { ok: false, reason: "wrong-scheme" };
  }
  // `new URL("schoolx://message?…")` puts "message" in `hostname`.
  if (parsed.hostname !== MESSAGE_LINK_HOST) {
    return { ok: false, reason: "wrong-host" };
  }

  const channelId = parsed.searchParams.get("channel");
  const messageId = parsed.searchParams.get("id");
  if (!channelId) {
    return { ok: false, reason: "missing-channel" };
  }
  if (!messageId) {
    return { ok: false, reason: "missing-id" };
  }

  return {
    ok: true,
    value: {
      channelId,
      messageId,
      threadRootId: parsed.searchParams.get("thread") ?? null,
    },
  };
}

/**
 * Convenience: returns true if the given href is a supported message link.
 * Cheap pre-check used by the markdown renderer before parsing.
 */
export function isMessageLink(href: string | undefined | null): boolean {
  if (!href) return false;
  return READABLE_MESSAGE_LINK_PREFIXES.some(
    (prefix) =>
      href.startsWith(`${prefix}${MESSAGE_LINK_HOST}?`) ||
      href === `${prefix}${MESSAGE_LINK_HOST}`,
  );
}

type MessageLinkRenderInput = {
  href: string;
  label: string;
};

export type MessageLinkRenderTarget =
  | { kind: "pill"; link: ParsedMessageLink }
  | { kind: "label"; link: ParsedMessageLink }
  | { kind: "none" };

/**
 * Centralizes how markdown-rendered anchors map to message-link UI. Both
 * CommonMark autolinks (`<schoolx://message?...>`) and explicitly labeled links
 * arrive as anchors; autolinks have label === href and should render as pills,
 * while intentionally labeled links keep their label.
 */
export function resolveMessageLinkRenderTarget({
  href,
  label,
}: MessageLinkRenderInput): MessageLinkRenderTarget {
  if (!isMessageLink(href)) return { kind: "none" };

  const parsed = parseMessageLink(href);
  if (!parsed.ok) return { kind: "none" };

  return {
    kind: label === href ? "pill" : "label",
    link: parsed.value,
  };
}
