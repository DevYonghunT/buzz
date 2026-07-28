import assert from "node:assert/strict";
import test from "node:test";

import {
  DEEP_LINK_SCHEME,
  LEGACY_DEEP_LINK_SCHEME,
} from "../../../shared/product/index.ts";
import { openPopoverLink } from "./openPopoverLink.ts";

const CHANNEL = "f570339f-8f8a-4e08-a779-8d954aa44109";
const MESSAGE =
  "b04819ffc1f7c8ffb49c6d30b5899f470198264680d02e78894a658e30a9059f";

function makeSpies() {
  const external = [];
  const inApp = [];
  return {
    handlers: {
      openExternal: (url) => external.push(url),
      openMessageLink: (link) => inApp.push(link),
    },
    external,
    inApp,
  };
}

test("product-scheme message deep-link routes in-app, not the OS opener", () => {
  const { handlers, external, inApp } = makeSpies();
  openPopoverLink(
    `${DEEP_LINK_SCHEME}://message?channel=${CHANNEL}&id=${MESSAGE}`,
    handlers,
  );
  assert.equal(external.length, 0);
  assert.deepEqual(inApp, [
    { channelId: CHANNEL, messageId: MESSAGE, threadRootId: null },
  ]);
});

// A legacy link must route in-app too, not to the OS opener. Handing it to the
// OS would be the worst outcome: nothing has `buzz` registered when only
// SchoolX is installed, so the click would silently do nothing.
test("legacy-scheme message deep-link also routes in-app", () => {
  const { handlers, external, inApp } = makeSpies();
  openPopoverLink(
    `${LEGACY_DEEP_LINK_SCHEME}://message?channel=${CHANNEL}&id=${MESSAGE}`,
    handlers,
  );
  assert.equal(external.length, 0);
  assert.deepEqual(inApp, [
    { channelId: CHANNEL, messageId: MESSAGE, threadRootId: null },
  ]);
});

test("http(s) URLs go to the OS opener", () => {
  const { handlers, external, inApp } = makeSpies();
  openPopoverLink("https://example.com/path", handlers);
  assert.deepEqual(external, ["https://example.com/path"]);
  assert.equal(inApp.length, 0);
});

test("non-message deep-link URLs fall through to the OS opener", () => {
  const { handlers, external, inApp } = makeSpies();
  const url = `${DEEP_LINK_SCHEME}://channel?foo=bar`;
  openPopoverLink(url, handlers);
  assert.deepEqual(external, [url]);
  assert.equal(inApp.length, 0);
});

test("malformed message URL falls back to the OS opener", () => {
  const { handlers, external, inApp } = makeSpies();
  // Matches isMessageLink (starts with schoolx://message?) but is missing the
  // required channel/id params, so parse fails and we don't navigate in-app.
  const url = `${DEEP_LINK_SCHEME}://message?nope=1`;
  openPopoverLink(url, handlers);
  assert.deepEqual(external, [url]);
  assert.equal(inApp.length, 0);
});
