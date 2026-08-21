import assert from "node:assert/strict";
import test from "node:test";

import { createCodeSessionState } from "./codeSessionReducer.ts";
import { captureCodeAuthoritativeRefreshCompletion } from "./codeSessionStore.ts";

const scope = {
  communityId: "community-1",
  projectDtag: "project-1",
  repositoryIdentity: "a".repeat(64),
};

function refreshState() {
  return {
    ...createCodeSessionState(scope),
    runtimeStatus: {
      phase: "ready",
      generation: 7,
      executable: "/usr/local/bin/codex",
      version: "codex-cli 0.145.0",
      pid: 123,
      userAgent: "codex-test",
      codexHome: "/tmp/codex",
      platformFamily: "unix",
      platformOs: "macos",
      queuedNotifications: 0,
      lastError: null,
    },
    runtimeStatusRevision: 1,
    runtimeGeneration: 7,
    replay: {
      status: "truncated",
      subscriptionEpoch: 3,
      request: {
        scope,
        runtimeGeneration: 7,
        afterSequence: 0,
      },
      needsAuthoritativeRefresh: true,
      approvalStateIncomplete: true,
    },
  };
}

test("authoritative refresh completion requires a ready pending identity", () => {
  const state = refreshState();
  const actions = [];

  assert.equal(
    captureCodeAuthoritativeRefreshCompletion(
      () => ({
        ...state,
        replay: { ...state.replay, needsAuthoritativeRefresh: false },
      }),
      (action) => actions.push(action),
    ),
    null,
  );
  assert.equal(
    captureCodeAuthoritativeRefreshCompletion(
      () => ({
        ...state,
        runtimeStatus: { ...state.runtimeStatus, phase: "stopped" },
      }),
      (action) => actions.push(action),
    ),
    null,
  );
  assert.deepEqual(actions, []);
});

test("authoritative refresh completion is exact, fail-closed, and one-shot", () => {
  let current = refreshState();
  const actions = [];
  const capture = () =>
    captureCodeAuthoritativeRefreshCompletion(
      () => current,
      (action) => actions.push(action),
    );

  const staleEpoch = capture();
  assert.equal(staleEpoch.runtimeGeneration, 7);
  assert.equal(staleEpoch.subscriptionEpoch, 3);
  current = {
    ...current,
    replay: { ...current.replay, subscriptionEpoch: 4 },
  };
  assert.equal(staleEpoch.complete(), false);

  current = refreshState();
  const staleGeneration = capture();
  current = { ...current, runtimeGeneration: 8 };
  assert.equal(staleGeneration.complete(), false);

  current = refreshState();
  const exact = capture();
  assert.equal(exact.complete(), true);
  assert.equal(exact.complete(), false);
  assert.deepEqual(actions, [
    {
      type: "authoritativeRefreshCompleted",
      runtimeGeneration: 7,
      subscriptionEpoch: 3,
    },
  ]);
});
