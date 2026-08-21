import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { CodeModelSelector } from "./CodeModelSelector.tsx";

const catalog = {
  runtimeGeneration: 3,
  models: [
    {
      id: "gpt-fast-preset",
      model: "gpt-fast",
      displayName: "GPT Fast",
      description: "Fast everyday model",
      isDefault: true,
      defaultReasoningEffort: "medium",
      supportedReasoningEfforts: [
        { reasoningEffort: "low", description: "Faster" },
        { reasoningEffort: "medium", description: "Balanced" },
      ],
    },
  ],
  recentSelection: { model: "gpt-fast", reasoningEffort: "medium" },
};

function controller(overrides = {}) {
  return {
    catalog,
    choice: { model: "gpt-fast", reasoningEffort: "medium" },
    turnSelection: { model: "gpt-fast", reasoningEffort: "medium" },
    newThreadSelection: { model: "gpt-fast", reasoningEffort: "medium" },
    loading: false,
    saving: false,
    error: null,
    chooseModel() {},
    chooseReasoningEffort() {},
    revalidateCatalog() {},
    retry() {},
    seedOpenedThread() {},
    ...overrides,
  };
}

test("selector exposes named model and effort menu triggers", () => {
  const html = renderToStaticMarkup(
    React.createElement(CodeModelSelector, {
      controller: controller(),
      disabled: false,
    }),
  );
  assert.match(html, /data-testid="code-model-selector"/);
  assert.match(html, /aria-label="Model: GPT Fast"/);
  assert.match(html, /data-testid="code-reasoning-selector"/);
  assert.match(html, /aria-label="Reasoning effort: Medium"/);
  assert.doesNotMatch(html, /aria-disabled="true"/);
});

test("busy selectors stay focusable while exposing aria-disabled state", () => {
  const html = renderToStaticMarkup(
    React.createElement(CodeModelSelector, {
      controller: controller(),
      disabled: true,
    }),
  );
  assert.equal((html.match(/aria-disabled="true"/g) ?? []).length, 2);
  assert.equal((html.match(/ disabled=""/g) ?? []).length, 0);
  assert.match(html, /unavailable while this task is busy/);
});

test("unknown open authority stays visible but its effort menu is locked", () => {
  const html = renderToStaticMarkup(
    React.createElement(CodeModelSelector, {
      controller: controller({
        choice: { model: "retired-model", reasoningEffort: "legacy_effort" },
        turnSelection: null,
      }),
      disabled: false,
    }),
  );
  assert.match(html, /aria-label="Model: retired-model"/);
  assert.match(html, /aria-label="Reasoning effort: Legacy Effort"/);
  assert.equal((html.match(/aria-disabled="true"/g) ?? []).length, 1);
});

test("catalog failure renders nonblocking fallback copy and retry", () => {
  const html = renderToStaticMarkup(
    React.createElement(CodeModelSelector, {
      controller: controller({
        catalog: null,
        choice: null,
        turnSelection: null,
        newThreadSelection: null,
        error: "Model options are unavailable. Codex defaults will be used.",
      }),
      disabled: false,
    }),
  );
  assert.match(html, /aria-label="Model: Codex default"/);
  assert.match(html, /role="alert"/);
  assert.match(html, /Codex defaults will be used/);
  assert.match(html, />Retry</);
});
