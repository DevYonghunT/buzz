import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { CodePanelResizeHandle } from "./CodePanelResizeHandle.tsx";

test("Code pane resize handle exposes separator value and keyboard focus", () => {
  const html = renderToStaticMarkup(
    React.createElement(CodePanelResizeHandle, {
      ariaLabel: "Resize Tasks and conversation",
      growDirection: 1,
      max: 420,
      min: 200,
      onChange() {},
      testId: "tasks-resize",
      value: 280,
    }),
  );

  assert.match(html, /^<hr/);
  assert.match(html, /aria-orientation="vertical"/);
  assert.match(html, /aria-valuemin="200"/);
  assert.match(html, /aria-valuemax="420"/);
  assert.match(html, /aria-valuenow="280"/);
  assert.match(html, /tabindex="0"/);
});

test("right pane reports the divider position while naming its visible width", () => {
  const html = renderToStaticMarkup(
    React.createElement(CodePanelResizeHandle, {
      ariaLabel: "Resize conversation and Changes",
      growDirection: -1,
      max: 720,
      min: 320,
      onChange() {},
      testId: "changes-resize",
      value: 544,
    }),
  );

  assert.match(html, /aria-valuenow="496"/);
  assert.match(html, /aria-valuetext="544 pixels"/);
});
