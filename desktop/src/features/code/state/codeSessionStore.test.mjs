import assert from "node:assert/strict";
import test from "node:test";

import { createCodeSessionState } from "./codeSessionReducer.ts";
import {
  captureCodeAuthoritativeRefreshCompletion,
  reprobeCodeRuntimeStatus,
} from "./codeSessionStore.ts";

const scope = {
  communityId: "community-1",
  projectDtag: "project-1",
  repositoryIdentity: "a".repeat(64),
};

test("manual runtime refresh re-probes before reading updated status", async () => {
  const calls = [];
  const probe = {
    available: true,
    executable: "C:\\Users\\user\\AppData\\Roaming\\npm\\codex.cmd",
    version: "codex-cli 0.149.0",
    error: null,
  };
  const status = {
    phase: "stopped",
    generation: 0,
    executable: probe.executable,
    version: probe.version,
    pid: null,
    userAgent: null,
    codexHome: null,
    platformFamily: null,
    platformOs: null,
    queuedNotifications: 0,
    lastError: null,
  };

  const result = await reprobeCodeRuntimeStatus({
    async probeCodeRuntime() {
      calls.push("probe");
      return probe;
    },
    async getCodeRuntimeStatus() {
      calls.push("status");
      return status;
    },
  });

  assert.deepEqual(calls, ["probe", "status"]);
  assert.deepEqual(result, { probe, status });
});

test("manual runtime refresh does not read stale status when probing fails", async () => {
  let statusReads = 0;

  await assert.rejects(
    reprobeCodeRuntimeStatus({
      async probeCodeRuntime() {
        throw new Error("probe transport failed");
      },
      async getCodeRuntimeStatus() {
        statusReads += 1;
        throw new Error("status should not be read");
      },
    }),
    /probe transport failed/,
  );
  assert.equal(statusReads, 0);
});

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
