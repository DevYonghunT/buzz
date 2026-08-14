import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { TauriInvokeError } from "@/shared/api/tauri.ts";

import {
  CODE_WORKSPACE_COMMAND_CONTRACT,
  CODE_WORKSPACE_EVENT_NAME,
  createCodeWorkspaceApi,
  getCodeThreadStartError,
} from "./codeWorkspace.ts";
import {
  CodeApprovalResponseInputSchema,
  CodeRepositoryInspectInputSchema,
  CodeRuntimeEventsInputSchema,
  CodeThreadBindingRecoverInputSchema,
  CodeThreadListInputSchema,
  CodeThreadPreparationListInputSchema,
  CodeThreadResumeInputSchema,
  CodeThreadStartInputSchema,
  CodeTurnInterruptInputSchema,
  CodeTurnStartInputSchema,
  CodeTurnSteerInputSchema,
  CodeWorktreeDescriptorSchema,
  CodeWorktreePrepareInputSchema,
  codeWorkspaceOutputSchemas,
} from "./schemas.ts";
import {
  CODE_APPROVAL_DECISIONS,
  CODE_APPROVAL_RESPONSE_TYPES,
  CODE_EXECUTION_MODES,
  CODE_PERMISSION_SCOPES,
  CODE_RUNTIME_PHASES,
  CODE_THREAD_PREPARATION_STATES,
  CODE_WORKSPACE_APPROVAL_REQUEST_KINDS,
  CODE_WORKSPACE_NOTIFICATION_KINDS,
} from "./types.ts";

const contract = JSON.parse(
  readFileSync(
    new URL(
      "../../../../src-tauri/src/code_workspace/fixtures/tauri-contract-v1.json",
      import.meta.url,
    ),
    "utf8",
  ),
);

const wire = JSON.parse(
  readFileSync(
    new URL(
      "../../../../src-tauri/src/code_workspace/fixtures/codex-0.145.0-wire.json",
      import.meta.url,
    ),
    "utf8",
  ),
);

test("frontend constants match the frozen native command and event contract", () => {
  assert.deepEqual(CODE_WORKSPACE_COMMAND_CONTRACT, contract.commands);
  assert.equal(CODE_WORKSPACE_EVENT_NAME, contract.eventName);
  assert.deepEqual(CODE_EXECUTION_MODES, contract.enums.executionMode);
  assert.deepEqual(
    CODE_THREAD_PREPARATION_STATES,
    contract.enums.preparationState,
  );
  assert.deepEqual(CODE_RUNTIME_PHASES, contract.enums.runtimePhase);
  assert.deepEqual(CODE_APPROVAL_DECISIONS, contract.enums.approvalDecision);
  assert.deepEqual(CODE_PERMISSION_SCOPES, contract.enums.permissionScope);
  assert.deepEqual(
    CODE_APPROVAL_RESPONSE_TYPES,
    contract.enums.approvalResponseType,
  );
});

test("frontend event kinds match every frozen Codex notification and approval", () => {
  assert.deepEqual(
    CODE_WORKSPACE_NOTIFICATION_KINDS,
    wire.notifications.map((notification) => notification.method),
  );
  assert.deepEqual(
    CODE_WORKSPACE_APPROVAL_REQUEST_KINDS,
    wire.approvals.map((approval) => approval.request.method),
  );
});

test("all native output fixtures pass strict frontend decoders", () => {
  assert.deepEqual(
    Object.keys(codeWorkspaceOutputSchemas).sort(),
    Object.keys(contract.outputs).sort(),
  );
  for (const [name, schema] of Object.entries(codeWorkspaceOutputSchemas)) {
    assert.deepEqual(
      schema.parse(contract.outputs[name]),
      contract.outputs[name],
    );
  }

  assert.equal(contract.outputs.eventWithoutIds.threadId, null);
  assert.equal(contract.outputs.eventWithoutIds.turnId, null);
  assert.equal(contract.outputs.eventWithoutIds.itemId, null);
  assert.equal(contract.outputs.unitResponse, null);
  assert.equal(
    "recoveryThreadBaseline" in contract.outputs.preparationPublicBaseline,
    false,
  );
});

test("strict frontend input decoders consume the native fixtures", () => {
  const inputs = contract.strictInputs;
  const parsed = [
    CodeRepositoryInspectInputSchema.parse(inputs.repositoryInspect),
    CodeWorktreePrepareInputSchema.parse(inputs.worktreePrepare),
    CodeThreadPreparationListInputSchema.parse(inputs.threadPreparationList),
    CodeThreadStartInputSchema.parse(inputs.threadStart),
    CodeThreadBindingRecoverInputSchema.parse(inputs.threadBindingRecover),
    CodeThreadListInputSchema.parse(inputs.threadList),
    CodeThreadResumeInputSchema.parse(inputs.threadResume),
    CodeTurnStartInputSchema.parse(inputs.turnStart),
    CodeTurnSteerInputSchema.parse(inputs.turnSteer),
    CodeTurnInterruptInputSchema.parse(inputs.turnInterrupt),
    CodeApprovalResponseInputSchema.parse(inputs.approvalDecision),
    CodeApprovalResponseInputSchema.parse(inputs.approvalPermissions),
    CodeRuntimeEventsInputSchema.parse(contract.invocations.runtimeEvents),
    CodeWorktreeDescriptorSchema.parse(
      contract.invocations.worktreeStatus.descriptor,
    ),
  ];

  assert.equal(parsed.length, 14);
  assert.throws(() =>
    CodeRepositoryInspectInputSchema.parse({
      ...inputs.repositoryInspect,
      unknown: true,
    }),
  );
  assert.throws(() =>
    CodeThreadStartInputSchema.parse({ ...inputs.threadStart, unknown: true }),
  );
  assert.throws(() =>
    CodeRuntimeEventsInputSchema.parse({
      ...contract.invocations.runtimeEvents,
      runtimeGeneration: null,
    }),
  );
  assert.throws(() =>
    CodeApprovalResponseInputSchema.parse({
      ...inputs.approvalPermissions,
      requestId: Number.MAX_SAFE_INTEGER + 1,
    }),
  );
});

function outputForCommand(command) {
  switch (command) {
    case "code_runtime_probe":
      return contract.outputs.runtimeProbe;
    case "code_runtime_start":
    case "code_runtime_stop":
    case "code_runtime_status":
      return contract.outputs.runtimeStatus;
    case "code_runtime_events":
      return contract.outputs.eventBacklog;
    case "code_repository_inspect":
      return contract.outputs.repositoryDescriptor;
    case "code_worktree_prepare":
      return contract.outputs.preparedWorktree;
    case "code_worktree_status":
      return contract.outputs.worktreeStatus;
    case "code_thread_preparations_list":
      return contract.outputs.preparationList;
    case "code_threads_list":
      return contract.outputs.threadsPage;
    case "code_thread_start":
    case "code_thread_binding_recover":
    case "code_thread_resume":
      return contract.outputs.boundThreadOpen;
    case "code_turn_start":
    case "code_turn_steer":
      return contract.outputs.turnSummary;
    case "code_turn_interrupt":
    case "code_approval_respond":
      return contract.outputs.unitResponse;
    default:
      throw new Error(`Unexpected command: ${command}`);
  }
}

test("all typed wrappers invoke the exact native command and argument shape", async () => {
  const invocations = [];
  let eventHandler = null;
  let unlistened = false;
  const api = createCodeWorkspaceApi({
    async invoke(command, args) {
      invocations.push({ command, args });
      return outputForCommand(command);
    },
    async listen(eventName, handler) {
      assert.equal(eventName, contract.eventName);
      eventHandler = handler;
      return () => {
        unlistened = true;
      };
    },
  });

  await api.probeCodeRuntime();
  await api.startCodeRuntime();
  await api.stopCodeRuntime();
  await api.getCodeRuntimeStatus();
  await api.getCodeRuntimeEvents(contract.invocations.runtimeEvents);
  await api.inspectCodeRepository(contract.strictInputs.repositoryInspect);
  await api.prepareCodeWorktree(contract.strictInputs.worktreePrepare);
  await api.getCodeWorktreeStatus(
    contract.invocations.worktreeStatus.descriptor,
  );
  await api.listCodeThreadPreparations(
    contract.strictInputs.threadPreparationList,
  );
  await api.listCodeThreads(contract.strictInputs.threadList);
  await api.startCodeThread(contract.strictInputs.threadStart);
  await api.recoverCodeThreadBinding(
    contract.strictInputs.threadBindingRecover,
  );
  await api.resumeCodeThread(contract.strictInputs.threadResume);
  await api.startCodeTurn(contract.strictInputs.turnStart);
  await api.steerCodeTurn(contract.strictInputs.turnSteer);
  await api.interruptCodeTurn(contract.strictInputs.turnInterrupt);
  await api.respondToCodeApproval(contract.strictInputs.approvalDecision);

  assert.deepEqual(
    invocations.map(({ command, args }) => ({
      name: command,
      topLevelArgs: Object.keys(args ?? {}).sort(),
    })),
    contract.commands,
  );
  assert.deepEqual(invocations[4].args, contract.invocations.runtimeEvents);
  assert.deepEqual(invocations[5].args, {
    input: contract.strictInputs.repositoryInspect,
  });
  assert.deepEqual(invocations[6].args, {
    input: contract.strictInputs.worktreePrepare,
  });
  assert.deepEqual(invocations[7].args, contract.invocations.worktreeStatus);

  let received = null;
  const listenerErrors = [];
  const unlisten = await api.listenForCodeWorkspaceEvents(
    (event) => {
      received = event;
    },
    (error) => listenerErrors.push(error),
  );
  eventHandler({ payload: contract.outputs.event });
  assert.deepEqual(received, contract.outputs.event);
  eventHandler({ payload: { malformed: true } });
  assert.equal(listenerErrors.length, 1);
  unlisten();
  assert.equal(unlistened, true);
});

test("listen-and-replay buffers live events until the replay snapshot arrives", async () => {
  let eventHandler = null;
  let replayRequested = false;
  const buffered = {
    ...contract.outputs.event,
    sequence: contract.outputs.event.sequence + 1,
  };
  const live = {
    ...contract.outputs.event,
    sequence: contract.outputs.event.sequence + 2,
  };
  const api = createCodeWorkspaceApi({
    async invoke(command) {
      assert.equal(command, "code_runtime_events");
      replayRequested = true;
      eventHandler({ payload: buffered });
      return contract.outputs.eventBacklog;
    },
    async listen(_eventName, handler) {
      eventHandler = handler;
      return () => {};
    },
  });

  let replayBatch = null;
  const liveEvents = [];
  await api.listenAndReplayCodeWorkspaceEvents(
    contract.invocations.runtimeEvents,
    {
      onReplay(batch) {
        replayBatch = batch;
      },
      onEvent(event, subscriptionEpoch) {
        liveEvents.push({ event, subscriptionEpoch });
      },
      onError(error) {
        assert.fail(error);
      },
    },
    { subscriptionEpoch: 42 },
  );

  assert.equal(replayRequested, true);
  assert.deepEqual(replayBatch, {
    subscriptionEpoch: 42,
    request: contract.invocations.runtimeEvents,
    backlog: contract.outputs.eventBacklog,
    bufferedEvents: [buffered],
    bufferTruncated: false,
  });
  eventHandler({ payload: live });
  assert.deepEqual(liveEvents, [{ event: live, subscriptionEpoch: 42 }]);
});

test("listen-and-replay retries a generation change from sequence zero", async () => {
  const requests = [];
  let eventHandler = null;
  const api = createCodeWorkspaceApi({
    async invoke(command, args) {
      assert.equal(command, "code_runtime_events");
      requests.push(args);
      if (requests.length === 1) {
        return {
          runtimeGeneration: 8,
          latestSequence: 1,
          truncated: true,
          events: [],
        };
      }
      return {
        runtimeGeneration: 8,
        latestSequence: 2,
        truncated: false,
        events: [
          {
            ...contract.outputs.event,
            runtimeGeneration: 8,
            sequence: 1,
          },
        ],
      };
    },
    async listen(_eventName, handler) {
      eventHandler = handler;
      return () => {};
    },
  });
  let replay = null;
  await api.listenAndReplayCodeWorkspaceEvents(
    contract.invocations.runtimeEvents,
    {
      onReplay(batch) {
        replay = batch;
      },
      onEvent() {},
      onError(error) {
        assert.fail(error);
      },
    },
    { subscriptionEpoch: 5 },
  );
  assert.ok(eventHandler);
  assert.deepEqual(requests, [
    contract.invocations.runtimeEvents,
    {
      scope: contract.invocations.runtimeEvents.scope,
      runtimeGeneration: 8,
      afterSequence: 0,
    },
  ]);
  assert.equal(replay.backlog.truncated, false);
  assert.equal(replay.request.runtimeGeneration, 8);
});

test("aborting before listener registration resolves still cleans it up", async () => {
  let resolveListen;
  let unlistenCount = 0;
  const controller = new AbortController();
  const api = createCodeWorkspaceApi({
    async invoke() {
      assert.fail("replay must not start after abort");
    },
    listen() {
      return new Promise((resolve) => {
        resolveListen = resolve;
      });
    },
  });
  const pending = api.listenAndReplayCodeWorkspaceEvents(
    contract.invocations.runtimeEvents,
    { onReplay() {}, onEvent() {}, onError() {} },
    { subscriptionEpoch: 9, signal: controller.signal },
  );
  controller.abort();
  await assert.rejects(pending, { name: "AbortError" });
  resolveListen(() => {
    unlistenCount += 1;
  });
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(unlistenCount, 1);
});

test("replay failures unlisten and never deliver late state", async () => {
  let eventHandler = null;
  let unlistenCount = 0;
  const api = createCodeWorkspaceApi({
    async invoke() {
      throw new Error("replay failed");
    },
    async listen(_eventName, handler) {
      eventHandler = handler;
      return () => {
        unlistenCount += 1;
      };
    },
  });
  const replayEvents = [];
  await assert.rejects(
    api.listenAndReplayCodeWorkspaceEvents(
      contract.invocations.runtimeEvents,
      {
        onReplay(batch) {
          replayEvents.push(batch);
        },
        onEvent(incoming) {
          replayEvents.push(incoming);
        },
        onError(error) {
          assert.fail(error);
        },
      },
      { subscriptionEpoch: 11 },
    ),
    /replay failed/,
  );
  eventHandler({ payload: contract.outputs.event });
  assert.equal(unlistenCount, 1);
  assert.deepEqual(replayEvents, []);
});

test("malformed replay events preserve the decode error and clean up once", async () => {
  let eventHandler = null;
  let unlistenCount = 0;
  let resolveReplay;
  const api = createCodeWorkspaceApi({
    invoke() {
      return new Promise((resolve) => {
        resolveReplay = resolve;
      });
    },
    async listen(_eventName, handler) {
      eventHandler = handler;
      return () => {
        unlistenCount += 1;
      };
    },
  });
  const reported = [];
  const pending = api.listenAndReplayCodeWorkspaceEvents(
    contract.invocations.runtimeEvents,
    {
      onReplay() {},
      onEvent() {},
      onError(error) {
        reported.push(error);
        throw new Error("consumer error must not mask decode failure");
      },
    },
    { subscriptionEpoch: 12 },
  );
  await Promise.resolve();
  await Promise.resolve();
  while (resolveReplay === undefined) {
    await new Promise((resolve) => setImmediate(resolve));
  }
  eventHandler({ payload: { malformed: true } });
  resolveReplay(contract.outputs.eventBacklog);
  await assert.rejects(pending, (error) => error?.name === "ZodError");
  assert.equal(reported.length, 1);
  assert.equal(unlistenCount, 1);
});

test("malformed events reject even while the replay invoke is still pending", async () => {
  let eventHandler = null;
  let unlistenCount = 0;
  const api = createCodeWorkspaceApi({
    invoke() {
      return new Promise(() => {});
    },
    async listen(_eventName, handler) {
      eventHandler = handler;
      return () => {
        unlistenCount += 1;
      };
    },
  });
  const pending = api.listenAndReplayCodeWorkspaceEvents(
    contract.invocations.runtimeEvents,
    { onReplay() {}, onEvent() {}, onError() {} },
    { subscriptionEpoch: 13 },
  );
  await Promise.resolve();
  await Promise.resolve();
  eventHandler({ payload: { malformed: true } });
  await assert.rejects(pending, (error) => error?.name === "ZodError");
  assert.equal(unlistenCount, 1);
});

test("events from another scope are ignored before full decoding", async () => {
  let eventHandler = null;
  const received = [];
  const errors = [];
  const api = createCodeWorkspaceApi({
    async invoke() {
      return contract.outputs.eventBacklog;
    },
    async listen(_eventName, handler) {
      eventHandler = handler;
      return () => {};
    },
  });
  const dispose = await api.listenAndReplayCodeWorkspaceEvents(
    contract.invocations.runtimeEvents,
    {
      onReplay() {},
      onEvent(incoming) {
        received.push(incoming);
      },
      onError(error) {
        errors.push(error);
      },
    },
    { subscriptionEpoch: 14 },
  );
  eventHandler({
    payload: {
      scope: {
        ...contract.invocations.runtimeEvents.scope,
        projectDtag: "other-project",
      },
      malformed: true,
    },
  });
  assert.deepEqual(received, []);
  assert.deepEqual(errors, []);
  dispose();
});

test("structured thread-start errors are recovered from Tauri payloads", () => {
  const error = new TauriInvokeError(
    contract.outputs.threadStartError.message,
    contract.outputs.threadStartError,
  );
  assert.deepEqual(
    getCodeThreadStartError(error),
    contract.outputs.threadStartError,
  );
  assert.equal(getCodeThreadStartError(new Error("plain failure")), null);
});
