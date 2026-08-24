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
  CodeModelsCatalogSchema,
  CodeModelSelectionInputSchema,
  CodeModelSelectionSchema,
  CodePermissionDisplaySchema,
  CodeRepositoryInspectInputSchema,
  CodeRuntimeEventsInputSchema,
  CodeTerminalEventSchema,
  CodeTerminalOpenInputSchema,
  CodeTerminalResizeInputSchema,
  CodeTerminalStdinInputSchema,
  CodeTerminalTerminateInputSchema,
  CodeThreadBindingRecoverInputSchema,
  CodeThreadChangesSchema,
  CodeThreadChangesInputSchema,
  CodeThreadForkInputSchema,
  CodeThreadLifecycleMutationResultSchema,
  CodeThreadListInputSchema,
  CodeThreadLifecycleMutationInputSchema,
  CodeThreadPreparationSchema,
  CodeThreadRenameInputSchema,
  CodeThreadPreparationListInputSchema,
  CodeThreadResumeInputSchema,
  CodeThreadStartInputSchema,
  CodeThreadsPageSchema,
  CodeTurnInterruptInputSchema,
  CodeTurnStartInputSchema,
  CodeTurnSteerInputSchema,
  CodeWorkspaceEventSchema,
  CodeWorktreeDescriptorSchema,
  CodeWorktreeInventoryRowSchema,
  CodeWorktreePrepareInputSchema,
  CodeWorktreeRemovalReceiptSchema,
  CodeWorktreeRemoveInputSchema,
  CodeWorktreesListInputSchema,
  codeWorkspaceOutputSchemas,
} from "./schemas.ts";
import {
  CODE_APPROVAL_DECISIONS,
  CODE_APPROVAL_RESPONSE_TYPES,
  CODE_EXECUTION_MODES,
  CODE_PERMISSION_INTENTS,
  CODE_PERMISSION_SCOPES,
  CODE_RUNTIME_PHASES,
  CODE_THREAD_CHANGE_STATUSES,
  CODE_THREAD_LIFECYCLE_STATES,
  CODE_THREAD_PREPARATION_OPERATIONS,
  CODE_THREAD_PREPARATION_STATES,
  CODE_WORKTREE_INVENTORY_BLOCKERS,
  CODE_WORKSPACE_APPROVAL_REQUEST_KINDS,
  CODE_WORKSPACE_NOTIFICATION_KINDS,
} from "./types.ts";
import {
  CodeGitAcknowledgeReceiptSchema,
  CodeGitCommitReceiptSchema,
  CodeGitIndexMutationReceiptSchema,
  CodeGitReconcileResultSchema,
  CodeGitStatusSchema,
} from "./codeGitSchemas.ts";

const allCodeWorkspaceOutputSchemas = {
  ...codeWorkspaceOutputSchemas,
  gitStatus: CodeGitStatusSchema,
  gitStageReceipt: CodeGitIndexMutationReceiptSchema,
  gitUnstageReceipt: CodeGitIndexMutationReceiptSchema,
  gitCommitReceipt: CodeGitCommitReceiptSchema,
  gitReconcile: CodeGitReconcileResultSchema,
  gitAcknowledge: CodeGitAcknowledgeReceiptSchema,
};

const contract = JSON.parse(
  readFileSync(
    new URL(
      "../../../../src-tauri/src/code_workspace/fixtures/tauri-contract-v1.json",
      import.meta.url,
    ),
    "utf8",
  ),
);

const removalGateContract = JSON.parse(
  readFileSync(
    new URL(
      "../../../../src-tauri/src/code_workspace/fixtures/worktree-removal-gates-v1.json",
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

function validForkOutput(input = contract.strictInputs.threadFork) {
  const threadId = "thread-forked";
  const executionRoot = "/native/forked-root";
  return {
    binding: {
      ...input.scope,
      codexThreadId: threadId,
      executionMode: "worktree",
      executionRoot,
      baseRef: "c".repeat(40),
      worktreeId: "11111111-2222-4333-8444-555555555555",
    },
    thread: {
      ...contract.outputs.threadSummary,
      id: threadId,
      sessionId: "session-forked",
      forkedFromId: input.threadId,
      ephemeral: false,
      cwd: executionRoot,
    },
    instructionSources: ["AGENTS.md"],
    model: contract.outputs.boundThreadOpen.model,
    reasoningEffort: contract.outputs.boundThreadOpen.reasoningEffort,
  };
}

const inventoryScope = {
  communityId: "inventory-community",
  projectDtag: "inventory-project",
  repositoryIdentity: "d".repeat(64),
};
const inventoryBaseRef = "e".repeat(40);

function validInventoryRow(overrides = {}) {
  return {
    scope: inventoryScope,
    authority: {
      type: "binding",
      threadId: "thread-archived",
      lifecycle: "archived",
    },
    descriptor: {
      executionMode: "worktree",
      repositoryIdentity: inventoryScope.repositoryIdentity,
      executionRoot: "/native/inventory-root",
      baseRef: inventoryBaseRef,
      worktreeId: "inventory-worktree",
    },
    inspection: {
      status: "available",
      headCommit: inventoryBaseRef,
      branch: null,
      dirty: false,
    },
    preserved: true,
    canRemove: false,
    blockers: ["mergeProofUnavailable"],
    ...overrides,
  };
}

function validRemovableInventoryRow(overrides = {}) {
  return validInventoryRow({
    canRemove: true,
    blockers: [],
    inspection: {
      status: "available",
      headCommit: "f".repeat(40),
      branch: null,
      dirty: false,
    },
    ...overrides,
  });
}

test("frontend constants match the frozen native command and event contract", () => {
  assert.deepEqual(CODE_WORKSPACE_COMMAND_CONTRACT, contract.commands);
  assert.equal(CODE_WORKSPACE_EVENT_NAME, contract.eventName);
  assert.deepEqual(CODE_EXECUTION_MODES, contract.enums.executionMode);
  assert.deepEqual(
    CODE_THREAD_PREPARATION_STATES,
    contract.enums.preparationState,
  );
  assert.deepEqual(
    CODE_THREAD_PREPARATION_OPERATIONS,
    contract.enums.preparationOperation,
  );
  assert.deepEqual(CODE_RUNTIME_PHASES, contract.enums.runtimePhase);
  assert.deepEqual(
    CODE_THREAD_CHANGE_STATUSES,
    contract.enums.threadChangeStatus,
  );
  assert.deepEqual(
    CODE_THREAD_LIFECYCLE_STATES,
    contract.enums.threadLifecycle,
  );
  assert.deepEqual(
    CODE_WORKTREE_INVENTORY_BLOCKERS,
    contract.enums.worktreeInventoryBlocker,
  );
  assert.deepEqual(CODE_APPROVAL_DECISIONS, contract.enums.approvalDecision);
  assert.deepEqual(CODE_PERMISSION_SCOPES, contract.enums.permissionScope);
  assert.deepEqual(CODE_PERMISSION_INTENTS, contract.enums.permissionIntent);
  assert.deepEqual(
    CODE_APPROVAL_RESPONSE_TYPES,
    contract.enums.approvalResponseType,
  );
});

test("worktree removal gates expose only the exact public command and receipt", () => {
  const gateOrder = [
    "mergedAuthority",
    "durableRemovalJournal",
    "bindingTranscriptSemantics",
    "pinnedDeletionBoundary",
  ];
  assert.equal(removalGateContract.version, 1);
  assert.equal(
    removalGateContract.status,
    "authorityProofJournalPhysicalRemovalImplementedPublicSurfaceOpen",
  );
  assert.deepEqual(removalGateContract.gateOrder, gateOrder);
  assert.deepEqual(Object.keys(removalGateContract.gates), gateOrder);
  for (const gate of gateOrder) {
    assert.equal(removalGateContract.gates[gate].state, "implementedClosed");
  }
  assert.equal(removalGateContract.currentStoreVersion, 4);
  assert.equal(
    removalGateContract.gates.mergedAuthority.storeCollection,
    "mergeTargets",
  );
  assert.equal(
    removalGateContract.gates.mergedAuthority.captureImplemented,
    true,
  );
  assert.equal(
    removalGateContract.gates.mergedAuthority.proofImplemented,
    true,
  );
  assert.equal(
    removalGateContract.gates.mergedAuthority.publicProofSurface,
    false,
  );
  assert.deepEqual(removalGateContract.futureSurface, {
    commandName: "code_worktree_remove",
    topLevelArgs: ["input"],
    inputFields: ["scope", "threadId"],
    operationId: "nativeCanonicalUuid",
    registered: true,
    frontendMethodExposed: true,
    buttonRendered: true,
  });
  assert.deepEqual(removalGateContract.futureReceipt.fields, [
    "removalId",
    "scope",
    "threadId",
    "worktreeId",
    "headCommit",
    "mergedIntoRef",
    "mergedIntoCommit",
    "transcriptDisposition",
    "executionDisposition",
  ]);
  assert.deepEqual(removalGateContract.currentInventory, {
    preserved: true,
    canRemove: "eligibleOnly",
    archivedBlocker: "mergeProofUnavailableUnlessProven",
  });
  assert.deepEqual(removalGateContract.gates.durableRemovalJournal.states, [
    "claimed",
    "removing",
    "removed",
  ]);
  assert.equal(
    removalGateContract.gates.durableRemovalJournal.casImplemented,
    true,
  );
  assert.equal(
    removalGateContract.gates.durableRemovalJournal.physicalMutationImplemented,
    true,
  );
  assert.equal(
    removalGateContract.gates.bindingTranscriptSemantics.transcriptDisposition,
    "preserved",
  );
  assert.equal(
    removalGateContract.gates.pinnedDeletionBoundary
      .currentPinnedGitHelperReusable,
    false,
  );
  assert.equal(
    removalGateContract.gates.pinnedDeletionBoundary
      .physicalMutationImplemented,
    true,
  );
  assert.equal(
    removalGateContract.gates.pinnedDeletionBoundary.publicRemovalEntrypoint,
    true,
  );
  assert.equal(
    removalGateContract.gates.pinnedDeletionBoundary.proofRefCleanup,
    "durableExactCompareAndDelete",
  );
  assert.equal(
    removalGateContract.gates.pinnedDeletionBoundary.manifestIdentityPolicy,
    "deviceInodeBirthTimeAndSupportedGeneration",
  );
  assert.equal(
    removalGateContract.gates.pinnedDeletionBoundary.manifestPathPolicy,
    "sameMountHandleRelativeNamedDirectoryAndFileIdentity",
  );
  assert.equal(
    removalGateContract.gates.pinnedDeletionBoundary
      .removedCleanupOfflinePolicy,
    "preserveSidecarAndDefer",
  );
  assert.equal(
    removalGateContract.gates.pinnedDeletionBoundary.gitAdminLockPolicy,
    "lockedMarkerOrLockfileReject",
  );
  assert.equal(
    removalGateContract.gates.pinnedDeletionBoundary.objectStoragePolicy,
    "primarySameMountNoFollowNoAlternates",
  );
  assert.equal(
    removalGateContract.gates.pinnedDeletionBoundary.refStoragePolicy,
    "filesBackendWithLooseProofRef",
  );
  assert.equal(
    removalGateContract.gates.pinnedDeletionBoundary.mountBoundaryPolicy,
    "sameMountIdentityNoNestedMounts",
  );
  assert.equal(
    removalGateContract.gates.pinnedDeletionBoundary.proofRefRepresentation,
    "directLooseRegularNoFollow",
  );
  assert.equal(
    removalGateContract.gates.pinnedDeletionBoundary.proofRefDurability,
    "referenceFileAndDirectoryFsync",
  );
  assert.equal(
    removalGateContract.gates.pinnedDeletionBoundary.partialDeletionPolicy,
    "knownPrefixOnly",
  );
  assert.deepEqual(
    removalGateContract.gates.pinnedDeletionBoundary.startupRecoveryBefore,
    [
      "runtimeEmitterStart",
      "lifecycleReconciliation",
      "startRecovery",
      "forkRecovery",
    ],
  );
  assert.deepEqual(
    removalGateContract.gates.pinnedDeletionBoundary
      .pendingRemovalConflictGates,
    ["archivedRename", "turnInterrupt"],
  );
  assert.equal(
    removalGateContract.acceptanceCases.deletionBoundary.includes(
      "manifestSidecarReplacementFailsClosed",
    ),
    true,
  );
  assert.equal(
    removalGateContract.acceptanceCases.deletionBoundary.includes(
      "lockedGitAdminRejects",
    ),
    true,
  );
  assert.equal(
    removalGateContract.acceptanceCases.deletionBoundary.includes(
      "offlineCommonDirCleanupDefers",
    ),
    true,
  );
  assert.equal(
    removalGateContract.physicalRemovalOrder.at(-2),
    "compareDeleteExactProofRef",
  );
  assert.equal(
    removalGateContract.physicalRemovalOrder.at(-1),
    "durablyRetireManifestSidecar",
  );

  const destructiveWorktree =
    /^code_worktree.*(?:remove|delete|destroy|cleanup|clean|prune|purge|discard)/i;
  assert.deepEqual(
    CODE_WORKSPACE_COMMAND_CONTRACT.filter(({ name }) =>
      destructiveWorktree.test(name),
    ).map(({ name }) => name),
    ["code_worktree_remove"],
  );
  assert.deepEqual(
    CodeWorktreeRemoveInputSchema.parse(contract.strictInputs.worktreeRemove),
    contract.strictInputs.worktreeRemove,
  );
  assert.deepEqual(
    CodeWorktreeRemovalReceiptSchema.parse(
      contract.outputs.worktreeRemovalReceipt,
    ),
    contract.outputs.worktreeRemovalReceipt,
  );

  const api = createCodeWorkspaceApi({
    async invoke() {
      throw new Error("not called");
    },
    async listen() {
      return () => {};
    },
  });
  assert.deepEqual(
    Object.keys(api).filter((name) =>
      /(?:remove|delete|destroy|cleanup|clean|prune|purge|discard).*worktree|worktree.*(?:remove|delete|destroy|cleanup|clean|prune|purge|discard)/i.test(
        name,
      ),
    ),
    ["removeCodeWorktree"],
  );

  const listInput = { scope: inventoryScope };
  for (const [field, value] of [
    ["threadId", "thread-archived"],
    ["executionRoot", "/caller/substitution"],
    ["worktreeId", "caller-worktree"],
    ["descriptor", validInventoryRow().descriptor],
    ["lifecycle", "archived"],
    ["canRemove", true],
    ["targetRef", "refs/heads/main"],
    ["headCommit", inventoryBaseRef],
    ["removalOperationId", "caller-operation"],
  ]) {
    assert.equal(
      CodeWorktreesListInputSchema.safeParse({
        ...listInput,
        [field]: value,
      }).success,
      false,
      `inventory input accepted removal field ${field}`,
    );
  }

  const removeInput = contract.strictInputs.worktreeRemove;
  for (const [field, value] of [
    ["executionRoot", "/caller/substitution"],
    ["worktreeId", "caller-worktree"],
    ["descriptor", validInventoryRow().descriptor],
    ["targetRef", "refs/heads/main"],
    ["headCommit", inventoryBaseRef],
    ["targetCommit", inventoryBaseRef],
    ["mergeProof", { forged: true }],
    ["removalId", "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"],
    ["force", true],
  ]) {
    assert.equal(
      CodeWorktreeRemoveInputSchema.safeParse({
        ...removeInput,
        [field]: value,
      }).success,
      false,
      `remove input accepted caller authority ${field}`,
    );
  }
});

test("frontend event kinds match every frozen Codex notification and approval", () => {
  assert.deepEqual(
    CODE_WORKSPACE_NOTIFICATION_KINDS,
    wire.notifications.map((notification) => notification.method),
  );
  assert.deepEqual(CODE_WORKSPACE_APPROVAL_REQUEST_KINDS, [
    ...new Set(wire.approvals.map((approval) => approval.request.method)),
  ]);
});

test("all native output fixtures pass strict frontend decoders", () => {
  assert.deepEqual(
    Object.keys(allCodeWorkspaceOutputSchemas).sort(),
    Object.keys(contract.outputs).sort(),
  );
  for (const [name, schema] of Object.entries(allCodeWorkspaceOutputSchemas)) {
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
  const lifecycleRow = contract.outputs.threadsPage.data[0];
  assert.equal(Object.keys(lifecycleRow.binding).length, 8);
  assert.equal("lifecycle" in lifecycleRow.binding, false);
  assert.equal(typeof lifecycleRow.lifecycle, "string");
});

test("model catalog and selection preserve their exact public shapes", async () => {
  const catalog = contract.outputs.modelCatalog;
  const selection = contract.outputs.modelSelection;
  assert.deepEqual(CodeModelsCatalogSchema.parse(catalog), catalog);
  assert.deepEqual(CodeModelSelectionSchema.parse(selection), selection);
  assert.deepEqual(Object.keys(catalog).sort(), [
    "models",
    "recentSelection",
    "runtimeGeneration",
  ]);
  assert.deepEqual(Object.keys(catalog.models[0]).sort(), [
    "defaultReasoningEffort",
    "description",
    "displayName",
    "id",
    "isDefault",
    "model",
    "supportedReasoningEfforts",
  ]);
  assert.equal(
    CodeModelsCatalogSchema.safeParse({ ...catalog, hidden: false }).success,
    false,
  );
  assert.equal(
    CodeModelsCatalogSchema.safeParse({
      ...catalog,
      models: [],
      recentSelection: null,
    }).success,
    false,
  );
  assert.equal(
    CodeModelsCatalogSchema.safeParse({
      ...catalog,
      recentSelection: {
        model: catalog.models[0].model,
        reasoningEffort: "unsupported",
      },
    }).success,
    false,
  );

  const api = createCodeWorkspaceApi({
    async invoke(command, args) {
      assert.equal(command, "code_model_selection_set");
      assert.deepEqual(args, { input: contract.strictInputs.modelSelection });
      return { ...selection, reasoningEffort: "medium" };
    },
    async listen() {
      return () => {};
    },
  });
  await assert.rejects(
    api.setCodeModelSelection(contract.strictInputs.modelSelection),
    /must match its exact request/,
  );
});

test("strict frontend input decoders consume the native fixtures", () => {
  const inputs = contract.strictInputs;
  const parsed = [
    CodeTerminalOpenInputSchema.parse(inputs.terminalOpen),
    CodeTerminalResizeInputSchema.parse(inputs.terminalResize),
    CodeTerminalStdinInputSchema.parse(inputs.terminalStdin),
    CodeTerminalTerminateInputSchema.parse(inputs.terminalTerminate),
    CodeRepositoryInspectInputSchema.parse(inputs.repositoryInspect),
    CodeWorktreePrepareInputSchema.parse(inputs.worktreePrepare),
    CodeThreadPreparationListInputSchema.parse(inputs.threadPreparationList),
    CodeWorktreesListInputSchema.parse(inputs.worktreesList),
    CodeWorktreeRemoveInputSchema.parse(inputs.worktreeRemove),
    CodeThreadStartInputSchema.parse(inputs.threadStart),
    CodeThreadBindingRecoverInputSchema.parse(inputs.threadBindingRecover),
    CodeThreadListInputSchema.parse(inputs.threadList),
    CodeThreadForkInputSchema.parse(inputs.threadFork),
    CodeThreadLifecycleMutationInputSchema.parse(inputs.threadArchive),
    CodeThreadLifecycleMutationInputSchema.parse(inputs.threadUnarchive),
    CodeThreadRenameInputSchema.parse(inputs.threadRename),
    CodeThreadChangesInputSchema.parse(inputs.threadChanges),
    CodeThreadResumeInputSchema.parse(inputs.threadResume),
    CodeModelSelectionInputSchema.parse(inputs.modelSelection),
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

  assert.equal(parsed.length, 26);
  assert.throws(() =>
    CodeTerminalOpenInputSchema.parse({
      ...inputs.terminalOpen,
      cols: 0,
    }),
  );
  assert.throws(() =>
    CodeTerminalOpenInputSchema.parse({
      ...inputs.terminalOpen,
      cols: 1_001,
    }),
  );
  assert.throws(() =>
    CodeTerminalStdinInputSchema.parse({
      ...inputs.terminalStdin,
      data: [256],
    }),
  );
  assert.throws(() =>
    CodeTerminalTerminateInputSchema.parse({
      ...inputs.terminalTerminate,
      executionRoot: "/caller/substitution",
    }),
  );
  assert.throws(() =>
    CodeTerminalTerminateInputSchema.parse({
      ...inputs.terminalTerminate,
      sessionId: "not-a-session-uuid",
    }),
  );
  assert.throws(() =>
    CodeTerminalTerminateInputSchema.parse({
      ...inputs.terminalTerminate,
      sessionId: inputs.terminalTerminate.sessionId.toUpperCase(),
    }),
  );
  assert.throws(() =>
    CodeRepositoryInspectInputSchema.parse({
      ...inputs.repositoryInspect,
      unknown: true,
    }),
  );
  assert.equal(
    CodeWorktreePrepareInputSchema.parse({
      ...inputs.worktreePrepare,
      executionMode: "local",
    }).executionMode,
    "local",
  );
  for (const [field, value] of [
    ["path", "/caller/substitution"],
    ["headCommit", "f".repeat(40)],
    ["ref", "refs/heads/caller"],
    ["argv", ["git", "clean", "-fd"]],
    ["dirty", true],
  ]) {
    assert.throws(() =>
      CodeWorktreePrepareInputSchema.parse({
        ...inputs.worktreePrepare,
        [field]: value,
      }),
    );
  }
  assert.throws(() =>
    CodeThreadStartInputSchema.parse({ ...inputs.threadStart, unknown: true }),
  );
  assert.throws(() =>
    CodeModelSelectionInputSchema.parse({
      ...inputs.modelSelection,
      unknown: true,
    }),
  );
  assert.throws(() =>
    CodeModelSelectionInputSchema.parse({
      ...inputs.modelSelection,
      reasoningEffort: " ",
    }),
  );
  assert.throws(() =>
    CodeThreadForkInputSchema.parse({
      ...inputs.threadFork,
      executionRoot: "/caller/substitution",
    }),
  );
  assert.throws(() =>
    CodeThreadForkInputSchema.parse({ ...inputs.threadFork, threadId: "" }),
  );
  for (const name of [
    "",
    " rename",
    "rename ",
    "rename\nnext",
    "x".repeat(129),
  ]) {
    assert.throws(() =>
      CodeThreadRenameInputSchema.parse({ ...inputs.threadRename, name }),
    );
  }
  assert.equal(
    CodeThreadRenameInputSchema.parse({
      ...inputs.threadRename,
      name: "😀".repeat(128),
    }).name,
    "😀".repeat(128),
  );
  assert.throws(() =>
    CodeThreadRenameInputSchema.parse({
      ...inputs.threadRename,
      unknown: true,
    }),
  );
  for (const input of [inputs.threadArchive, inputs.threadUnarchive]) {
    const { threadId: _threadId, ...withoutThreadId } = input;
    assert.throws(() =>
      CodeThreadLifecycleMutationInputSchema.parse(withoutThreadId),
    );
    assert.throws(() =>
      CodeThreadLifecycleMutationInputSchema.parse({
        ...input,
        executionRoot: "/caller/substitution",
      }),
    );
    assert.throws(() =>
      CodeThreadLifecycleMutationInputSchema.parse({
        ...input,
        lifecycle: "active",
      }),
    );
    assert.throws(() =>
      CodeThreadLifecycleMutationInputSchema.parse({
        ...input,
        scope: { ...input.scope, unknown: true },
      }),
    );
  }
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
  assert.throws(() =>
    CodeApprovalResponseInputSchema.parse({
      ...inputs.approvalPermissions,
      response: {
        ...inputs.approvalPermissions.response,
        permissions: { network: { enabled: true } },
      },
    }),
  );
  for (const approval of wire.approvals) {
    assert.deepEqual(
      CodeApprovalResponseInputSchema.parse(approval.responseInput),
      approval.responseInput,
    );
  }
});

test("thread lifecycle stays outside the frozen eight-field binding", () => {
  const page = contract.outputs.threadsPage;
  const row = page.data[0];
  const mutation = contract.outputs.threadLifecycleMutation;
  assert.equal(Object.keys(row.binding).length, 8);
  assert.equal("lifecycle" in row.binding, false);
  assert.equal(Object.keys(mutation.binding).length, 8);

  for (const invalidRow of [
    (({ lifecycle: _lifecycle, ...withoutLifecycle }) => withoutLifecycle)(row),
    { ...row, lifecycle: "future-state" },
    { ...row, lifecycle: row.lifecycle, operationId: "private-journal-id" },
    {
      ...row,
      binding: { ...row.binding, lifecycle: row.lifecycle },
    },
  ]) {
    assert.equal(
      CodeThreadsPageSchema.safeParse({ ...page, data: [invalidRow] }).success,
      false,
    );
  }
  for (const invalidMutation of [
    (({ lifecycle: _lifecycle, ...withoutLifecycle }) => withoutLifecycle)(
      mutation,
    ),
    { ...mutation, lifecycle: "future-state" },
    { ...mutation, operationId: "private-journal-id" },
    {
      ...mutation,
      binding: { ...mutation.binding, lifecycle: mutation.lifecycle },
    },
  ]) {
    assert.equal(
      CodeThreadLifecycleMutationResultSchema.safeParse(invalidMutation)
        .success,
      false,
    );
  }
});

test("thread preparations keep operation and fork source exact", () => {
  const start = contract.outputs.preparationPublicBaseline;
  assert.equal(start.operation, "start");
  assert.equal(start.sourceThreadId, null);
  assert.deepEqual(CodeThreadPreparationSchema.parse(start), start);

  const fork = {
    ...start,
    executionMode: "worktree",
    worktreeId: "11111111-2222-4333-8444-555555555555",
    operation: "fork",
    sourceThreadId: "thread-source",
  };
  assert.deepEqual(CodeThreadPreparationSchema.parse(fork), fork);
  for (const invalid of [
    { ...start, sourceThreadId: "thread-source" },
    { ...fork, sourceThreadId: null },
    { ...fork, sourceThreadId: "" },
    { ...fork, executionMode: "local", worktreeId: null },
    { ...fork, operation: "future-operation" },
  ]) {
    assert.equal(CodeThreadPreparationSchema.safeParse(invalid).success, false);
  }
});

test("fork adapter rejects mismatched source and managed destination", async () => {
  const input = contract.strictInputs.threadFork;
  const valid = validForkOutput(input);
  const validApi = createCodeWorkspaceApi({
    async invoke(command) {
      assert.equal(command, "code_thread_fork");
      return valid;
    },
    async listen() {
      return () => {};
    },
  });
  assert.deepEqual(await validApi.forkCodeThread(input), valid);

  const invalidResults = [
    {
      ...valid,
      binding: { ...valid.binding, projectDtag: "other-project" },
    },
    {
      ...valid,
      binding: { ...valid.binding, codexThreadId: "other-thread" },
    },
    {
      ...valid,
      binding: { ...valid.binding, codexThreadId: input.threadId },
      thread: { ...valid.thread, id: input.threadId },
    },
    { ...valid, thread: { ...valid.thread, forkedFromId: "other-thread" } },
    { ...valid, thread: { ...valid.thread, cwd: "/native/other-root" } },
    {
      ...valid,
      binding: {
        ...valid.binding,
        executionMode: "local",
        worktreeId: null,
      },
    },
    { ...valid, thread: { ...valid.thread, ephemeral: true } },
  ];

  for (const result of invalidResults) {
    const api = createCodeWorkspaceApi({
      async invoke(command) {
        assert.equal(command, "code_thread_fork");
        return result;
      },
      async listen() {
        return () => {};
      },
    });
    await assert.rejects(
      api.forkCodeThread(input),
      /must match its exact source and managed destination/,
    );
  }
});

test("lifecycle adapter rejects mismatched scope, thread, and stable result", async () => {
  const input = contract.strictInputs.threadArchive;
  const valid = {
    ...contract.outputs.threadLifecycleMutation,
    lifecycle: "archived",
  };
  const invalidResults = [
    {
      ...valid,
      binding: { ...valid.binding, projectDtag: "other-project" },
    },
    {
      ...valid,
      binding: { ...valid.binding, codexThreadId: "other-thread" },
    },
    {
      ...valid,
      thread: { ...contract.outputs.threadSummary, id: "other-thread" },
    },
    { ...valid, lifecycle: "active" },
  ];

  for (const result of invalidResults) {
    assert.equal(
      CodeThreadLifecycleMutationResultSchema.safeParse(result).success,
      true,
    );
    const api = createCodeWorkspaceApi({
      async invoke(command) {
        assert.equal(command, "code_thread_archive");
        return result;
      },
      async listen() {
        return () => {};
      },
    });
    await assert.rejects(
      api.archiveCodeThread(input),
      /must match its exact bound-thread request/,
    );
  }
});

test("terminal events preserve exact ownership, bytes, and lifecycle shape", () => {
  assert.deepEqual(
    CodeTerminalEventSchema.parse(contract.outputs.terminalOutputEvent),
    contract.outputs.terminalOutputEvent,
  );
  assert.deepEqual(
    CodeTerminalEventSchema.parse(contract.outputs.terminalExitEvent),
    contract.outputs.terminalExitEvent,
  );
  for (const invalid of [
    {
      ...contract.outputs.terminalOutputEvent,
      data: [0, 256],
    },
    {
      ...contract.outputs.terminalExitEvent,
      exitCode: null,
    },
    {
      ...contract.outputs.terminalExitEvent,
      type: "error",
    },
    {
      ...contract.outputs.terminalOutputEvent,
      executionRoot: "/caller/substitution",
    },
  ]) {
    assert.equal(CodeTerminalEventSchema.safeParse(invalid).success, false);
  }
});

test("terminal channel decodes before delivery and rejects stale ownership", async () => {
  let channel = null;
  const received = [];
  const api = createCodeWorkspaceApi({
    async invoke(command) {
      assert.equal(command, "code_terminal_open");
      return contract.outputs.terminalSession;
    },
    async listen() {
      return () => {};
    },
    createChannel(handler) {
      channel = { onmessage: handler };
      return channel;
    },
  });

  await api.openCodeTerminal(contract.strictInputs.terminalOpen, (event) => {
    received.push(event);
  });
  assert.throws(() => channel.onmessage({ malformed: true }));
  assert.throws(() =>
    channel.onmessage({
      ...contract.outputs.terminalOutputEvent,
      scope: {
        ...contract.outputs.terminalOutputEvent.scope,
        projectDtag: "other-project",
      },
    }),
  );
  assert.throws(() =>
    channel.onmessage({
      ...contract.outputs.terminalOutputEvent,
      sessionId: "stale-session",
    }),
  );
  assert.throws(() =>
    channel.onmessage({
      ...contract.outputs.terminalOutputEvent,
      sequence: 0,
    }),
  );
  channel.onmessage(contract.outputs.terminalOutputEvent);
  assert.throws(() =>
    channel.onmessage({
      ...contract.outputs.terminalOutputEvent,
      data: [62, 32],
    }),
  );
  channel.onmessage(contract.outputs.terminalExitEvent);
  assert.throws(() =>
    channel.onmessage({
      ...contract.outputs.terminalOutputEvent,
      sequence: 3,
    }),
  );
  assert.deepEqual(received, [
    contract.outputs.terminalOutputEvent,
    contract.outputs.terminalExitEvent,
  ]);
});

test("terminal channel buffers output until the exact native session is known", async () => {
  let channel = null;
  let resolveOpen = null;
  const received = [];
  const api = createCodeWorkspaceApi({
    invoke(command) {
      assert.equal(command, "code_terminal_open");
      return new Promise((resolve) => {
        resolveOpen = resolve;
      });
    },
    async listen() {
      return () => {};
    },
    createChannel(handler) {
      channel = { onmessage: handler };
      return channel;
    },
  });

  const pending = api.openCodeTerminal(
    contract.strictInputs.terminalOpen,
    (event) => received.push(event),
  );
  channel.onmessage(contract.outputs.terminalOutputEvent);
  assert.deepEqual(received, []);
  resolveOpen(contract.outputs.terminalSession);
  await pending;
  assert.deepEqual(received, [contract.outputs.terminalOutputEvent]);
});

test("thread changes enforce complete file metadata and exact list truncation", () => {
  const complete = {
    files: [
      {
        path: "src/changed.ts",
        status: "modified",
        binary: false,
        additions: 2,
        deletions: 1,
        patch: "@@ -1 +1 @@\n-old\n+new",
        truncated: false,
      },
    ],
    additions: 2,
    deletions: 1,
    commitBody: null,
    totalFiles: 1,
    filesTruncated: false,
  };
  assert.deepEqual(CodeThreadChangesSchema.parse(complete), complete);
  assert.deepEqual(
    CodeThreadChangesSchema.parse({
      ...complete,
      totalFiles: 2,
      filesTruncated: true,
    }),
    { ...complete, totalFiles: 2, filesTruncated: true },
  );

  for (const invalid of [
    { ...complete, totalFiles: 0 },
    { ...complete, totalFiles: 2 },
    { ...complete, filesTruncated: true },
    {
      ...complete,
      files: [{ ...complete.files[0], status: "renamed" }],
    },
    {
      ...complete,
      files: complete.files.map(({ binary: _binary, ...file }) => file),
    },
    { ...complete, additions: 3 },
    {
      ...complete,
      files: [complete.files[0], { ...complete.files[0] }],
      additions: 4,
      deletions: 2,
      totalFiles: 2,
    },
    {
      ...complete,
      files: [
        {
          ...complete.files[0],
          binary: true,
        },
      ],
    },
  ]) {
    assert.equal(CodeThreadChangesSchema.safeParse(invalid).success, false);
  }
});

test("permission events accept only typed display data, never raw authority", () => {
  const permissionDisplay = {
    grantable: true,
    network: { enabled: true },
    fileSystem: {
      entries: [
        {
          access: "write",
          path: { type: "path", path: "/native/stored-root/generated" },
        },
        {
          access: "read",
          path: {
            type: "globPattern",
            pattern: "/native/stored-root/**/*.rs",
          },
        },
        {
          access: "deny",
          path: {
            type: "special",
            value: { kind: "project_roots", subpath: ".git" },
          },
        },
      ],
      globScanMaxDepth: 12,
      read: ["/native/stored-root"],
      write: null,
    },
  };
  assert.deepEqual(
    CodePermissionDisplaySchema.parse(permissionDisplay),
    permissionDisplay,
  );
  const event = {
    scope: contract.strictInputs.approvalPermissions.scope,
    runtimeGeneration: 7,
    sequence: 1,
    threadId: "thread-1",
    turnId: "turn-1",
    itemId: "item-permissions",
    kind: "item/permissions/requestApproval",
    payload: {
      requestId: 9,
      approvalKind: "permissions",
      request: {
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "item-permissions",
        permissionDisplay,
      },
    },
  };
  assert.deepEqual(CodeWorkspaceEventSchema.parse(event), event);
  assert.throws(() =>
    CodeWorkspaceEventSchema.parse({
      ...event,
      payload: {
        ...event.payload,
        request: {
          ...event.payload.request,
          permissions: { network: { enabled: true } },
        },
      },
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
    case "code_models_list":
      return contract.outputs.modelCatalog;
    case "code_model_selection_set":
      return contract.outputs.modelSelection;
    case "code_terminal_open":
      return contract.outputs.terminalSession;
    case "code_terminal_resize":
    case "code_terminal_stdin":
    case "code_terminal_terminate":
      return contract.outputs.unitResponse;
    case "code_repository_inspect":
      return contract.outputs.repositoryDescriptor;
    case "code_worktree_prepare":
      return contract.outputs.preparedWorktree;
    case "code_worktree_status":
      return contract.outputs.worktreeStatus;
    case "code_worktrees_list":
      return contract.outputs.worktreeInventory;
    case "code_worktree_remove":
      return contract.outputs.worktreeRemovalReceipt;
    case "code_thread_preparations_list":
      return contract.outputs.preparationList;
    case "code_threads_list":
      return contract.outputs.threadsPage;
    case "code_thread_fork":
      return validForkOutput();
    case "code_thread_archive":
      return {
        ...contract.outputs.threadLifecycleMutation,
        lifecycle: "archived",
      };
    case "code_thread_unarchive":
      return {
        ...contract.outputs.threadLifecycleMutation,
        lifecycle: "active",
      };
    case "code_thread_rename":
      return contract.outputs.threadSummary;
    case "code_thread_changes":
      return contract.outputs.threadChanges;
    case "code_thread_git_status":
      return contract.outputs.gitStatus;
    case "code_thread_git_stage":
      return contract.outputs.gitStageReceipt;
    case "code_thread_git_unstage":
      return contract.outputs.gitUnstageReceipt;
    case "code_thread_git_commit":
      return contract.outputs.gitCommitReceipt;
    case "code_thread_git_reconcile":
      return contract.outputs.gitReconcile;
    case "code_thread_git_acknowledge":
      return contract.outputs.gitAcknowledge;
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
  let terminalChannel = null;
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
    createChannel(handler) {
      terminalChannel = { onmessage: handler };
      return terminalChannel;
    },
  });

  await api.probeCodeRuntime();
  await api.startCodeRuntime();
  await api.stopCodeRuntime();
  await api.getCodeRuntimeStatus();
  await api.getCodeRuntimeEvents(contract.invocations.runtimeEvents);
  await api.listCodeModels();
  await api.setCodeModelSelection(contract.strictInputs.modelSelection);
  const terminalEvents = [];
  await api.openCodeTerminal(contract.strictInputs.terminalOpen, (event) => {
    terminalEvents.push(event);
  });
  await api.resizeCodeTerminal(contract.strictInputs.terminalResize);
  await api.writeCodeTerminalStdin(contract.strictInputs.terminalStdin);
  await api.terminateCodeTerminal(contract.strictInputs.terminalTerminate);
  await api.inspectCodeRepository(contract.strictInputs.repositoryInspect);
  await api.prepareCodeWorktree(contract.strictInputs.worktreePrepare);
  await api.getCodeWorktreeStatus(
    contract.invocations.worktreeStatus.descriptor,
  );
  await api.listCodeWorktrees(contract.strictInputs.worktreesList);
  await api.removeCodeWorktree(contract.strictInputs.worktreeRemove);
  await api.listCodeThreadPreparations(
    contract.strictInputs.threadPreparationList,
  );
  await api.listCodeThreads(contract.strictInputs.threadList);
  await api.archiveCodeThread(contract.strictInputs.threadArchive);
  await api.unarchiveCodeThread(contract.strictInputs.threadUnarchive);
  await api.renameCodeThread(contract.strictInputs.threadRename);
  await api.getCodeThreadChanges(contract.strictInputs.threadChanges);
  await api.startCodeThread(contract.strictInputs.threadStart);
  await api.forkCodeThread(contract.strictInputs.threadFork);
  await api.recoverCodeThreadBinding(
    contract.strictInputs.threadBindingRecover,
  );
  await api.resumeCodeThread(contract.strictInputs.threadResume);
  await api.startCodeTurn(contract.strictInputs.turnStart);
  await api.steerCodeTurn(contract.strictInputs.turnSteer);
  await api.interruptCodeTurn(contract.strictInputs.turnInterrupt);
  await api.respondToCodeApproval(contract.strictInputs.approvalDecision);
  await api.getCodeThreadGitStatus(contract.strictInputs.gitStatus);
  await api.stageCodeThreadFile(contract.strictInputs.gitStage);
  await api.unstageCodeThreadFile(contract.strictInputs.gitStage);
  await api.commitCodeThread(contract.strictInputs.gitCommit);
  await api.reconcileCodeThreadGit(contract.strictInputs.gitStatus);
  await api.acknowledgeCodeThreadGit(contract.strictInputs.gitAcknowledge);

  assert.deepEqual(
    invocations.map(({ command, args }) => ({
      name: command,
      topLevelArgs: Object.keys(args ?? {}).sort(),
    })),
    contract.commands,
  );
  assert.deepEqual(invocations[4].args, contract.invocations.runtimeEvents);
  assert.deepEqual(
    invocations.find(({ command }) => command === "code_models_list")?.args,
    undefined,
  );
  assert.deepEqual(
    invocations.find(({ command }) => command === "code_model_selection_set")
      ?.args,
    { input: contract.strictInputs.modelSelection },
  );
  assert.deepEqual(invocations[11].args, {
    input: contract.strictInputs.repositoryInspect,
  });
  assert.deepEqual(invocations[12].args, {
    input: contract.strictInputs.worktreePrepare,
  });
  assert.deepEqual(invocations[13].args, contract.invocations.worktreeStatus);
  assert.deepEqual(
    invocations.find(({ command }) => command === "code_worktree_remove")?.args,
    { input: contract.strictInputs.worktreeRemove },
  );
  assert.deepEqual(
    invocations.find(({ command }) => command === "code_thread_fork")?.args,
    { input: contract.strictInputs.threadFork },
  );
  assert.deepEqual(
    invocations.find(({ command }) => command === "code_thread_archive")?.args,
    { input: contract.strictInputs.threadArchive },
  );
  assert.deepEqual(
    invocations.find(({ command }) => command === "code_thread_unarchive")
      ?.args,
    { input: contract.strictInputs.threadUnarchive },
  );
  assert.deepEqual(
    invocations.find(({ command }) => command === "code_thread_rename")?.args,
    { input: contract.strictInputs.threadRename },
  );

  terminalChannel.onmessage(contract.outputs.terminalOutputEvent);
  terminalChannel.onmessage(contract.outputs.terminalExitEvent);
  assert.deepEqual(terminalEvents, [
    contract.outputs.terminalOutputEvent,
    contract.outputs.terminalExitEvent,
  ]);

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
          checkpoint: {
            runtimeGeneration: 8,
            sequenceWatermark: 1,
            activeTurns: [],
            pendingApprovals: [],
          },
          events: [],
        };
      }
      return {
        runtimeGeneration: 8,
        latestSequence: 2,
        truncated: false,
        checkpoint: {
          runtimeGeneration: 8,
          sequenceWatermark: 2,
          activeTurns: [],
          pendingApprovals: [],
        },
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

test("managed worktree inventory schema closes preservation and blocker state", () => {
  const archived = validInventoryRow();
  const removable = validRemovableInventoryRow();
  const activeUnavailable = validInventoryRow({
    authority: {
      type: "binding",
      threadId: "thread-active",
      lifecycle: "active",
    },
    inspection: { status: "unavailable", error: "root disappeared" },
    blockers: ["activeBinding", "unavailableRoot"],
  });
  const preparation = validInventoryRow({
    authority: {
      type: "preparation",
      preparationId: "preparation-fork",
      operation: "fork",
      state: "starting",
      sourceThreadId: "thread-source",
    },
    inspection: {
      status: "available",
      headCommit: "f".repeat(40),
      branch: "schoolx/worktree",
      dirty: true,
    },
    blockers: [
      "unfinishedPreparation",
      "dirtyRoot",
      "branchAttached",
      "headDrift",
    ],
  });

  for (const row of [archived, removable, activeUnavailable, preparation]) {
    assert.deepEqual(CodeWorktreeInventoryRowSchema.parse(row), row);
  }

  const invalidRows = [
    { ...archived, preserved: false },
    { ...archived, canRemove: true },
    { ...archived, blockers: [] },
    { ...removable, canRemove: false },
    { ...removable, blockers: ["mergeProofUnavailable"] },
    {
      ...archived,
      blockers: ["mergeProofUnavailable", "mergeProofUnavailable"],
    },
    {
      ...activeUnavailable,
      blockers: ["unavailableRoot", "activeBinding"],
    },
    { ...archived, blockers: ["localCheckout", "mergeProofUnavailable"] },
    { ...archived, blockers: [] },
    {
      ...archived,
      descriptor: {
        ...archived.descriptor,
        executionMode: "local",
        worktreeId: null,
      },
    },
    {
      ...archived,
      descriptor: {
        ...archived.descriptor,
        repositoryIdentity: "0".repeat(64),
      },
    },
    {
      ...archived,
      inspection: { ...archived.inspection, error: "impossible" },
    },
    {
      ...activeUnavailable,
      inspection: {
        ...activeUnavailable.inspection,
        headCommit: inventoryBaseRef,
      },
    },
    { ...activeUnavailable, blockers: ["activeBinding"] },
    { ...preparation, blockers: ["unfinishedPreparation", "dirtyRoot"] },
    {
      ...preparation,
      authority: { ...preparation.authority, sourceThreadId: null },
    },
  ];
  for (const row of invalidRows) {
    assert.equal(CodeWorktreeInventoryRowSchema.safeParse(row).success, false);
  }
});

test("managed worktree inventory adapter sends exact scope and rejects foreign rows", async () => {
  const input = { scope: inventoryScope };
  assert.deepEqual(CodeWorktreesListInputSchema.parse(input), input);
  assert.throws(() =>
    CodeWorktreesListInputSchema.parse({
      ...input,
      executionRoot: "/caller/substitution",
    }),
  );

  const row = validInventoryRow();
  const calls = [];
  const api = createCodeWorkspaceApi({
    async invoke(command, args) {
      calls.push([command, args]);
      return [row];
    },
    async listen() {
      return () => {};
    },
  });
  assert.deepEqual(await api.listCodeWorktrees(input), [row]);
  assert.deepEqual(calls, [["code_worktrees_list", { input }]]);

  const foreignApi = createCodeWorkspaceApi({
    async invoke() {
      return [
        validInventoryRow({
          scope: { ...inventoryScope, projectDtag: "other-project" },
        }),
      ];
    },
    async listen() {
      return () => {};
    },
  });
  await assert.rejects(
    foreignApi.listCodeWorktrees(input),
    /must match the exact requested scope/,
  );

  for (const invalidRow of [
    {
      ...row,
      descriptor: {
        ...row.descriptor,
        executionMode: "local",
        worktreeId: null,
      },
    },
    { ...row, preserved: false },
    { ...row, canRemove: true },
    { ...row, blockers: ["localCheckout", "mergeProofUnavailable"] },
  ]) {
    const invalidApi = createCodeWorkspaceApi({
      async invoke() {
        return [invalidRow];
      },
      async listen() {
        return () => {};
      },
    });
    await assert.rejects(
      invalidApi.listCodeWorktrees(input),
      (error) => error?.name === "ZodError",
    );
  }
});

test("thread-list adapter rejects rows outside its exact requested scope", async () => {
  const input = contract.strictInputs.threadList;
  const page = contract.outputs.threadsPage;
  const calls = [];
  const api = createCodeWorkspaceApi({
    async invoke(command, args) {
      calls.push([command, args]);
      return structuredClone(page);
    },
    async listen() {
      return () => {};
    },
  });
  assert.deepEqual(await api.listCodeThreads(input), page);
  assert.deepEqual(calls, [["code_threads_list", { input }]]);

  const foreignPage = structuredClone(page);
  assert.ok(foreignPage.data.length > 0);
  foreignPage.data[0].binding.projectDtag = "project-foreign";
  const foreignApi = createCodeWorkspaceApi({
    async invoke() {
      return foreignPage;
    },
    async listen() {
      return () => {};
    },
  });
  await assert.rejects(
    foreignApi.listCodeThreads(input),
    /must match the exact requested scope/,
  );
});

test("managed worktree removal adapter sends only scope and thread and verifies its receipt", async () => {
  const input = contract.strictInputs.worktreeRemove;
  const receipt = contract.outputs.worktreeRemovalReceipt;
  assert.deepEqual(CodeWorktreeRemoveInputSchema.parse(input), input);
  const calls = [];
  const api = createCodeWorkspaceApi({
    async invoke(command, args) {
      calls.push([command, args]);
      return structuredClone(receipt);
    },
    async listen() {
      return () => {};
    },
  });

  assert.deepEqual(await api.removeCodeWorktree(input), receipt);
  assert.deepEqual(await api.removeCodeWorktree(input), receipt);
  assert.deepEqual(calls, [
    ["code_worktree_remove", { input }],
    ["code_worktree_remove", { input }],
  ]);

  for (const forbidden of [
    { ...input, executionRoot: "/caller/substitution" },
    { ...input, targetRef: "refs/heads/main" },
    { ...input, removalId: receipt.removalId },
    { ...input, force: true },
  ]) {
    await assert.rejects(
      api.removeCodeWorktree(forbidden),
      (error) => error?.name === "ZodError",
    );
  }

  for (const invalidReceipt of [
    { ...receipt, threadId: "thread-other" },
    {
      ...receipt,
      scope: { ...receipt.scope, projectDtag: "project-other" },
    },
  ]) {
    const mismatchedApi = createCodeWorkspaceApi({
      async invoke() {
        return invalidReceipt;
      },
      async listen() {
        return () => {};
      },
    });
    await assert.rejects(
      mismatchedApi.removeCodeWorktree(input),
      /must match its exact request/,
    );
  }

  for (const invalidReceipt of [
    {
      ...receipt,
      removalId: receipt.removalId.toUpperCase(),
    },
    {
      ...receipt,
      removalId: "aaaaaaaa-aaaa-1aaa-8aaa-aaaaaaaaaaaa",
    },
    {
      ...receipt,
      worktreeId: "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA",
    },
    { ...receipt, mergedIntoRef: "refs/heads/main.lock" },
    { ...receipt, mergedIntoRef: "refs/heads/team//main" },
    { ...receipt, mergedIntoRef: "refs/heads/main@{1}" },
    { ...receipt, mergedIntoRef: "refs/heads/main branch" },
  ]) {
    const invalidApi = createCodeWorkspaceApi({
      async invoke() {
        return invalidReceipt;
      },
      async listen() {
        return () => {};
      },
    });
    await assert.rejects(
      invalidApi.removeCodeWorktree(input),
      (error) => error?.name === "ZodError",
    );
  }

  const malformedApi = createCodeWorkspaceApi({
    async invoke() {
      return { ...receipt, executionRoot: "/native/private" };
    },
    async listen() {
      return () => {};
    },
  });
  await assert.rejects(
    malformedApi.removeCodeWorktree(input),
    (error) => error?.name === "ZodError",
  );
});
