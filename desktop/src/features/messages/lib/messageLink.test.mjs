import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  DEEP_LINK_SCHEME,
  LEGACY_DEEP_LINK_SCHEME,
} from "../../../shared/product/index.ts";
import {
  buildMessageLink,
  isMessageLink,
  parseMessageLink,
  resolveMessageLinkRenderTarget,
} from "./messageLink.ts";

const CHANNEL = "f570339f-8f8a-4e08-a779-8d954aa44109";
const MESSAGE =
  "b04819ffc1f7c8ffb49c6d30b5899f470198264680d02e78894a658e30a9059f";
const THREAD =
  "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";

const CURRENT = `${DEEP_LINK_SCHEME}://message`;
const LEGACY = `${LEGACY_DEEP_LINK_SCHEME}://message`;

test("buildMessageLink → parseMessageLink round-trips without thread", () => {
  const url = buildMessageLink({ channelId: CHANNEL, messageId: MESSAGE });
  assert.equal(url, `${CURRENT}?channel=${CHANNEL}&id=${MESSAGE}`);

  const parsed = parseMessageLink(url);
  assert.equal(parsed.ok, true);
  assert.deepEqual(parsed.ok && parsed.value, {
    channelId: CHANNEL,
    messageId: MESSAGE,
    threadRootId: null,
  });
});

test("buildMessageLink → parseMessageLink round-trips with thread", () => {
  const url = buildMessageLink({
    channelId: CHANNEL,
    messageId: MESSAGE,
    threadRootId: THREAD,
  });
  const parsed = parseMessageLink(url);
  assert.equal(parsed.ok, true);
  assert.deepEqual(parsed.ok && parsed.value, {
    channelId: CHANNEL,
    messageId: MESSAGE,
    threadRootId: THREAD,
  });
});

test("buildMessageLink treats null/empty thread as absent", () => {
  const a = buildMessageLink({
    channelId: CHANNEL,
    messageId: MESSAGE,
    threadRootId: null,
  });
  const b = buildMessageLink({
    channelId: CHANNEL,
    messageId: MESSAGE,
    threadRootId: "",
  });
  assert.equal(a, `${CURRENT}?channel=${CHANNEL}&id=${MESSAGE}`);
  assert.equal(b, `${CURRENT}?channel=${CHANNEL}&id=${MESSAGE}`);
});

test("buildMessageLink rejects missing required params", () => {
  assert.throws(() => buildMessageLink({ channelId: "", messageId: MESSAGE }));
  assert.throws(() => buildMessageLink({ channelId: CHANNEL, messageId: "" }));
});

// New links must always carry the SchoolX scheme. This is the half of the
// legacy policy that is *not* permissive: SchoolX reads `buzz://` but must
// never mint it, or it would keep seeding links the OS routes to Buzz.
test("buildMessageLink never emits the legacy scheme", () => {
  const url = buildMessageLink({ channelId: CHANNEL, messageId: MESSAGE });
  assert.ok(url.startsWith(`${DEEP_LINK_SCHEME}://`));
  assert.ok(!url.startsWith(`${LEGACY_DEEP_LINK_SCHEME}://`));
});

test("parseMessageLink rejects unsupported schemes", () => {
  const r = parseMessageLink(
    `https://example.com/?channel=${CHANNEL}&id=${MESSAGE}`,
  );
  assert.equal(r.ok, false);
  assert.equal(r.ok === false && r.reason, "wrong-scheme");
});

test("parseMessageLink rejects the product scheme with wrong host", () => {
  const r = parseMessageLink(
    `${DEEP_LINK_SCHEME}://connect?relay=wss://example.com`,
  );
  assert.equal(r.ok, false);
  assert.equal(r.ok === false && r.reason, "wrong-host");
});

test("parseMessageLink rejects missing channel", () => {
  const r = parseMessageLink(`${CURRENT}?id=${MESSAGE}`);
  assert.equal(r.ok, false);
  assert.equal(r.ok === false && r.reason, "missing-channel");
});

test("parseMessageLink rejects missing id", () => {
  const r = parseMessageLink(`${CURRENT}?channel=${CHANNEL}`);
  assert.equal(r.ok, false);
  assert.equal(r.ok === false && r.reason, "missing-id");
});

test("parseMessageLink rejects malformed URL strings", () => {
  const r = parseMessageLink("not a url");
  assert.equal(r.ok, false);
  assert.equal(r.ok === false && r.reason, "invalid-url");
});

// Message history written before the rename — or by a Buzz user in a shared
// community — still says `buzz://message`. Those links must keep resolving,
// otherwise old messages decay into unclickable 100-char strings.
test("parseMessageLink accepts legacy buzz://message links", () => {
  const r = parseMessageLink(`${LEGACY}?channel=${CHANNEL}&id=${MESSAGE}`);
  assert.equal(r.ok, true);
  assert.deepEqual(r.ok && r.value, {
    channelId: CHANNEL,
    messageId: MESSAGE,
    threadRootId: null,
  });
});

test("parseMessageLink reads thread from a legacy link too", () => {
  const r = parseMessageLink(
    `${LEGACY}?channel=${CHANNEL}&id=${MESSAGE}&thread=${THREAD}`,
  );
  assert.equal(r.ok, true);
  assert.equal(r.ok && r.value.threadRootId, THREAD);
});

test("isMessageLink matches both the product and legacy schemes", () => {
  assert.equal(
    isMessageLink(`${CURRENT}?channel=${CHANNEL}&id=${MESSAGE}`),
    true,
  );
  assert.equal(
    isMessageLink(`${LEGACY}?channel=${CHANNEL}&id=${MESSAGE}`),
    true,
  );
  assert.equal(isMessageLink(CURRENT), true);
  assert.equal(isMessageLink(LEGACY), true);
  assert.equal(
    isMessageLink(`${DEEP_LINK_SCHEME}://connect?relay=wss://x`),
    false,
  );
  assert.equal(
    isMessageLink(`${LEGACY_DEEP_LINK_SCHEME}://connect?relay=wss://x`),
    false,
  );
  assert.equal(isMessageLink("https://example.com"), false);
  assert.equal(isMessageLink(undefined), false);
  assert.equal(isMessageLink(""), false);
});

test("resolveMessageLinkRenderTarget distinguishes autolinks from labeled links", () => {
  const href = `${CURRENT}?channel=${CHANNEL}&id=${MESSAGE}`;

  assert.deepEqual(resolveMessageLinkRenderTarget({ href, label: href }), {
    kind: "pill",
    link: {
      channelId: CHANNEL,
      messageId: MESSAGE,
      threadRootId: null,
    },
  });
  assert.deepEqual(resolveMessageLinkRenderTarget({ href, label: "message" }), {
    kind: "label",
    link: {
      channelId: CHANNEL,
      messageId: MESSAGE,
      threadRootId: null,
    },
  });
  assert.deepEqual(
    resolveMessageLinkRenderTarget({
      href: "https://example.com",
      label: href,
    }),
    { kind: "none" },
  );
});

test("legacy links render as pills, same as current ones", () => {
  const href = `${LEGACY}?channel=${CHANNEL}&id=${MESSAGE}`;
  assert.deepEqual(resolveMessageLinkRenderTarget({ href, label: href }), {
    kind: "pill",
    link: {
      channelId: CHANNEL,
      messageId: MESSAGE,
      threadRootId: null,
    },
  });
});

// `remarkMessageLinks.ts` cannot import the product constants (it is loaded by
// `markdown.test.mjs` under a stricter loader and its pattern must stay a
// regex literal), so its scheme alternation is duplicated. Pin the duplication
// here: a future rename that updates one and not the other stops bare URLs in
// message text from turning into pills, silently.
test("remarkMessageLinks pattern covers exactly the readable schemes", () => {
  const here = dirname(fileURLToPath(import.meta.url));
  const source = readFileSync(join(here, "remarkMessageLinks.ts"), "utf8");
  const match = source.match(
    /const MESSAGE_URL_PATTERN = \/\(\?:([^)]+)\):\\\/\\\//,
  );
  assert.ok(match, "MESSAGE_URL_PATTERN must keep its (?:a|b):// shape");
  assert.deepEqual(match[1].split("|"), [
    DEEP_LINK_SCHEME,
    LEGACY_DEEP_LINK_SCHEME,
  ]);
});
