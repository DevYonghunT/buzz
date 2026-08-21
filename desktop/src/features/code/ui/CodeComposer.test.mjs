import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { CodeComposer } from "./CodeComposer.tsx";

test("Git handoff blocker disables the composer with a visible reason", () => {
  const reason =
    "The completed commit operation must be acknowledged before Code can continue.";
  const html = renderToStaticMarkup(
    React.createElement(CodeComposer, {
      active: false,
      canInterrupt: false,
      disabled: true,
      disabledReason: reason,
      async onInterrupt() {},
      async onSubmit() {
        return true;
      },
    }),
  );

  assert.match(html, /<textarea[^>]*disabled=""/);
  assert.match(html, /aria-describedby="code-composer-disabled-reason"/);
  assert.match(html, /role="status"/);
  assert.match(html, new RegExp(reason.replaceAll(".", "\\.")));
});
