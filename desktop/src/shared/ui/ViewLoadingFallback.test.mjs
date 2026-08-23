import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { ViewLoadingFallback } from "./ViewLoadingFallback.tsx";

test("project loading fallback is a named busy status with structural skeletons", () => {
  const html = renderToStaticMarkup(
    React.createElement(ViewLoadingFallback, {
      kind: "projects",
      label: "Loading project",
    }),
  );

  assert.match(html, /role="status"/);
  assert.match(html, /aria-busy="true"/);
  assert.match(html, /aria-label="Loading project"/);
  assert.match(html, /aria-hidden="true"/);
  assert.match(html, /t-skel-bar/);
});
