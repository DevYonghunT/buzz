import assert from "node:assert/strict";
import test from "node:test";

import { bakedAgentConfigIsComplete } from "./bakedOnboardingSkip.ts";

const entry = (key, value = "x") => ({ key, value, masked: false });

// Stands in for requiredCredentialEnvKeys from the agents feature.
const needsKey = (provider) =>
  provider === "anthropic" ? ["ANTHROPIC_API_KEY"] : [];

const COMPLETE = [
  entry("BUZZ_AGENT_PROVIDER", "anthropic"),
  entry("BUZZ_AGENT_MODEL", "claude-sonnet-5"),
  { key: "ANTHROPIC_API_KEY", value: "••••••", masked: true },
];

const without = (key) => COMPLETE.filter((e) => e.key !== key);

test("complete anthropic bake skips the steps", () => {
  assert.equal(bakedAgentConfigIsComplete(COMPLETE, needsKey), true);
});

// OSS builds bake nothing; the steps are the only way to configure an agent.
test("empty baked env does not skip", () => {
  assert.equal(bakedAgentConfigIsComplete([], needsKey), false);
  assert.equal(bakedAgentConfigIsComplete(undefined, needsKey), false);
});

test("provider without model does not skip", () => {
  assert.equal(
    bakedAgentConfigIsComplete(without("BUZZ_AGENT_MODEL"), needsKey),
    false,
  );
});

// The regression this guards: a build that bakes provider and model but no
// credential would skip straight past the only screen that could have
// surfaced the missing key, and every agent would fail on first use.
test("provider and model without the credential does not skip", () => {
  assert.equal(
    bakedAgentConfigIsComplete(without("ANTHROPIC_API_KEY"), needsKey),
    false,
  );
});

test("blank provider value does not skip", () => {
  const blank = [
    entry("BUZZ_AGENT_PROVIDER", "   "),
    entry("BUZZ_AGENT_MODEL", "m"),
    entry("ANTHROPIC_API_KEY"),
  ];
  assert.equal(bakedAgentConfigIsComplete(blank, needsKey), false);
});

// A provider that needs no env credential (CLI login, OAuth) is complete
// without one.
test("provider needing no credential skips on provider and model alone", () => {
  const oauth = [
    entry("BUZZ_AGENT_PROVIDER", "databricks"),
    entry("BUZZ_AGENT_MODEL", "m"),
  ];
  assert.equal(bakedAgentConfigIsComplete(oauth, needsKey), true);
});
