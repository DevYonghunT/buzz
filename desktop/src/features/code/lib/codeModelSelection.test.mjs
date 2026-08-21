import assert from "node:assert/strict";
import test from "node:test";

import {
  codeModelSelectionFromFreshOpen,
  codeModelSelectionFromOpen,
  codeReasoningEffortLabel,
  defaultCodeModelSelection,
  selectCodeModel,
  selectCodeReasoningEffort,
} from "./codeModelSelection.ts";

const catalog = {
  runtimeGeneration: 7,
  models: [
    {
      id: "fast-preset",
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
    {
      id: "deep-preset",
      model: "gpt-deep",
      displayName: "GPT Deep",
      description: "Deep reasoning model",
      isDefault: false,
      defaultReasoningEffort: "high",
      supportedReasoningEfforts: [
        { reasoningEffort: "medium", description: "Balanced" },
        { reasoningEffort: "high", description: "Thorough" },
      ],
    },
  ],
  recentSelection: null,
};

test("initial selection prefers recent, then advertised catalog default", () => {
  assert.deepEqual(defaultCodeModelSelection(catalog), {
    model: "gpt-fast",
    reasoningEffort: "medium",
  });
  assert.deepEqual(
    defaultCodeModelSelection({
      ...catalog,
      recentSelection: { model: "gpt-deep", reasoningEffort: "high" },
    }),
    { model: "gpt-deep", reasoningEffort: "high" },
  );
});

test("model changes retain supported effort and otherwise use model default", () => {
  assert.deepEqual(
    selectCodeModel(
      catalog,
      { model: "gpt-fast", reasoningEffort: "medium" },
      "gpt-deep",
    ),
    { model: "gpt-deep", reasoningEffort: "medium" },
  );
  assert.deepEqual(
    selectCodeModel(
      catalog,
      { model: "gpt-fast", reasoningEffort: "low" },
      "gpt-deep",
    ),
    { model: "gpt-deep", reasoningEffort: "high" },
  );
  assert.equal(selectCodeModel(catalog, null, "removed-model"), null);
});

test("effort changes only accept an advertised current-model value", () => {
  const current = { model: "gpt-fast", reasoningEffort: null };
  assert.deepEqual(selectCodeReasoningEffort(catalog, current, "low"), {
    model: "gpt-fast",
    reasoningEffort: "low",
  });
  assert.equal(selectCodeReasoningEffort(catalog, current, "high"), null);
});

test("open authority sends only a catalog-known exact pair", () => {
  assert.deepEqual(
    codeModelSelectionFromOpen(catalog, {
      model: "gpt-deep",
      reasoningEffort: "high",
    }),
    {
      choice: { model: "gpt-deep", reasoningEffort: "high" },
      turnSelection: { model: "gpt-deep", reasoningEffort: "high" },
    },
  );
  assert.deepEqual(
    codeModelSelectionFromOpen(catalog, {
      model: "gpt-deep",
      reasoningEffort: null,
    }),
    {
      choice: { model: "gpt-deep", reasoningEffort: null },
      turnSelection: null,
    },
  );
  assert.deepEqual(
    codeModelSelectionFromOpen(catalog, {
      model: "retired-model",
      reasoningEffort: "high",
    }),
    {
      choice: { model: "retired-model", reasoningEffort: "high" },
      turnSelection: null,
    },
  );
  assert.equal(
    codeModelSelectionFromOpen(null, {
      model: "gpt-deep",
      reasoningEffort: "high",
    }).turnSelection,
    null,
  );
});

test("fresh starts keep pending effort only when native opens that model", () => {
  const pending = { model: "gpt-deep", reasoningEffort: "medium" };
  assert.deepEqual(
    codeModelSelectionFromFreshOpen(
      catalog,
      { model: "gpt-deep", reasoningEffort: "high" },
      pending,
    ),
    { choice: pending, turnSelection: pending },
  );
  assert.deepEqual(
    codeModelSelectionFromFreshOpen(
      catalog,
      { model: "gpt-fast", reasoningEffort: "low" },
      pending,
    ),
    {
      choice: { model: "gpt-fast", reasoningEffort: "low" },
      turnSelection: { model: "gpt-fast", reasoningEffort: "low" },
    },
  );
});

test("reasoning labels keep unknown values visible", () => {
  assert.equal(codeReasoningEffortLabel("xhigh"), "Extra high");
  assert.equal(codeReasoningEffortLabel("very_deep"), "Very Deep");
  assert.equal(codeReasoningEffortLabel("---"), "---");
});
