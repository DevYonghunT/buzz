import { z } from "zod";

import {
  CodeModelIdentifierSchema,
  CodeModelsCatalogSchema,
  CodeModelSelectionSchema,
} from "./codeModelSchemas";
import {
  CODE_APPROVAL_DECISIONS,
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
  CODE_WORKSPACE_EVENT_KINDS,
} from "./types";

export {
  CodeModelIdentifierSchema,
  CodeModelOptionSchema,
  CodeModelsCatalogSchema,
  CodeModelSelectionInputSchema,
  CodeModelSelectionSchema,
  CodeReasoningEffortOptionSchema,
} from "./codeModelSchemas";

export const JsonValueSchema = z.json();
export const JsonObjectSchema = z.record(z.string(), JsonValueSchema);
const CodeSafeUnsignedIntegerSchema = z
  .number()
  .int()
  .nonnegative()
  .max(Number.MAX_SAFE_INTEGER);

export const CodeThreadBindingScopeSchema = z.strictObject({
  communityId: z.string(),
  projectDtag: z.string(),
  repositoryIdentity: z.string(),
});

export const CodeWorkspaceEventScopeSchema = z.object({
  scope: CodeThreadBindingScopeSchema,
});

export const CodeRuntimeProbeSchema = z.strictObject({
  available: z.boolean(),
  executable: z.string().nullable(),
  version: z.string().nullable(),
  error: z.string().nullable(),
});

export const CodeRuntimeStatusSchema = z.strictObject({
  phase: z.enum(CODE_RUNTIME_PHASES),
  generation: CodeSafeUnsignedIntegerSchema,
  executable: z.string().nullable(),
  version: z.string().nullable(),
  pid: z.number().int().nonnegative().nullable(),
  userAgent: z.string().nullable(),
  codexHome: z.string().nullable(),
  platformFamily: z.string().nullable(),
  platformOs: z.string().nullable(),
  queuedNotifications: CodeSafeUnsignedIntegerSchema,
  lastError: z.string().nullable(),
});

export const CodeWorktreeDescriptorSchema = z.strictObject({
  executionMode: z.enum(CODE_EXECUTION_MODES),
  repositoryIdentity: z.string(),
  executionRoot: z.string(),
  baseRef: z.string(),
  worktreeId: z.string().nullable(),
});

export const CodeManagedWorktreeDescriptorSchema = z.strictObject({
  executionMode: z.literal("worktree"),
  repositoryIdentity: z.string(),
  executionRoot: z.string(),
  baseRef: z.string(),
  worktreeId: z.string().min(1),
});

export const CodeWorktreeInventoryAuthoritySchema = z.discriminatedUnion(
  "type",
  [
    z.strictObject({
      type: z.literal("binding"),
      threadId: z.string().min(1),
      lifecycle: z.enum(CODE_THREAD_LIFECYCLE_STATES),
    }),
    z
      .strictObject({
        type: z.literal("preparation"),
        preparationId: z.string().min(1),
        operation: z.enum(CODE_THREAD_PREPARATION_OPERATIONS),
        state: z.enum(CODE_THREAD_PREPARATION_STATES),
        sourceThreadId: z.string().min(1).nullable(),
      })
      .superRefine((authority, context) => {
        if (
          (authority.operation === "start" &&
            authority.sourceThreadId !== null) ||
          (authority.operation === "fork" && authority.sourceThreadId === null)
        ) {
          context.addIssue({
            code: "custom",
            message:
              "Inventory preparation authority must match its start or fork source",
            path: ["sourceThreadId"],
          });
        }
      }),
  ],
);

export const CodeWorktreeInspectionSchema = z.discriminatedUnion("status", [
  z.strictObject({
    status: z.literal("available"),
    headCommit: z.string().min(1),
    branch: z.string().min(1).nullable(),
    dirty: z.boolean(),
  }),
  z.strictObject({
    status: z.literal("unavailable"),
    error: z.string().min(1),
  }),
]);

const CodeWorktreeInventoryBlockersSchema = z
  .array(z.enum(CODE_WORKTREE_INVENTORY_BLOCKERS))
  .superRefine((blockers, context) => {
    const seen = new Set<string>();
    let previousIndex = -1;
    for (const [index, blocker] of blockers.entries()) {
      const blockerIndex = CODE_WORKTREE_INVENTORY_BLOCKERS.indexOf(blocker);
      if (seen.has(blocker)) {
        context.addIssue({
          code: "custom",
          message: "Inventory blockers must be unique",
          path: [index],
        });
      }
      if (blockerIndex <= previousIndex) {
        context.addIssue({
          code: "custom",
          message: "Inventory blockers must follow native contract order",
          path: [index],
        });
      }
      seen.add(blocker);
      previousIndex = blockerIndex;
    }
  });

export const CodeWorktreeInventoryRowSchema = z
  .strictObject({
    scope: CodeThreadBindingScopeSchema,
    authority: CodeWorktreeInventoryAuthoritySchema,
    descriptor: CodeManagedWorktreeDescriptorSchema,
    inspection: CodeWorktreeInspectionSchema,
    preserved: z.literal(true),
    canRemove: z.boolean(),
    blockers: CodeWorktreeInventoryBlockersSchema,
  })
  .superRefine((row, context) => {
    if (row.scope.repositoryIdentity !== row.descriptor.repositoryIdentity) {
      context.addIssue({
        code: "custom",
        message: "Inventory descriptor must belong to its exact scope",
        path: ["descriptor", "repositoryIdentity"],
      });
    }

    const blockers = new Set(row.blockers);
    const archivedBinding =
      row.authority.type === "binding" &&
      row.authority.lifecycle === "archived";
    const mergeProofUnavailable = blockers.has("mergeProofUnavailable");
    const expectedBlockers = new Map<
      (typeof CODE_WORKTREE_INVENTORY_BLOCKERS)[number],
      boolean
    >([
      [
        "activeBinding",
        row.authority.type === "binding" &&
          row.authority.lifecycle === "active",
      ],
      [
        "lifecycleUnsettled",
        row.authority.type === "binding" &&
          row.authority.lifecycle !== "active" &&
          row.authority.lifecycle !== "archived",
      ],
      ["unfinishedPreparation", row.authority.type === "preparation"],
      ["localCheckout", false],
      ["unavailableRoot", row.inspection.status === "unavailable"],
      [
        "dirtyRoot",
        row.inspection.status === "available" && row.inspection.dirty,
      ],
      [
        "branchAttached",
        row.inspection.status === "available" && row.inspection.branch !== null,
      ],
      [
        "headDrift",
        row.inspection.status === "available" &&
          row.inspection.headCommit !== row.descriptor.baseRef &&
          (!archivedBinding || mergeProofUnavailable),
      ],
    ]);
    for (const [blocker, expected] of expectedBlockers) {
      if (blockers.has(blocker) !== expected) {
        context.addIssue({
          code: "custom",
          message: `Inventory blocker ${blocker} must match native row state`,
          path: ["blockers"],
        });
      }
    }
    if (mergeProofUnavailable && !archivedBinding) {
      context.addIssue({
        code: "custom",
        message:
          "Merge-proof availability applies only to archived binding rows",
        path: ["blockers"],
      });
    }
    if (
      archivedBinding &&
      row.inspection.status === "unavailable" &&
      !mergeProofUnavailable
    ) {
      context.addIssue({
        code: "custom",
        message:
          "An unavailable archived root cannot carry positive merge proof",
        path: ["blockers"],
      });
    }
    if (row.canRemove !== (row.blockers.length === 0)) {
      context.addIssue({
        code: "custom",
        message:
          "Inventory removal eligibility must match an empty blocker set",
        path: ["canRemove"],
      });
    }
    if (
      row.canRemove &&
      (row.authority.type !== "binding" ||
        row.authority.lifecycle !== "archived" ||
        row.inspection.status !== "available")
    ) {
      context.addIssue({
        code: "custom",
        message:
          "Only an available archived binding can be eligible for removal",
        path: ["canRemove"],
      });
    }
  });

export const CodeRepositoryDescriptorSchema = z.strictObject({
  repositoryRoot: z.string(),
  gitCommonDir: z.string(),
  repositoryIdentity: z.string(),
});

export const CodeRepositoryInspectInputSchema = z.strictObject({
  repositoryRoot: z.string(),
  baseRef: z.string(),
});

export const CodeWorktreePrepareResultSchema = z.strictObject({
  repository: CodeRepositoryDescriptorSchema,
  descriptor: CodeWorktreeDescriptorSchema,
  headCommit: z.string(),
  branch: z.string().nullable(),
  dirty: z.boolean(),
});

export const CodePreparedWorktreeSchema = z.strictObject({
  preparationId: z.string(),
  scope: CodeThreadBindingScopeSchema,
  worktree: CodeWorktreePrepareResultSchema,
});

export const CodeWorktreeStatusSchema = z.strictObject({
  descriptor: CodeWorktreeDescriptorSchema,
  headCommit: z.string(),
  branch: z.string().nullable(),
  dirty: z.boolean(),
});

export const CodeThreadBindingSchema = z.strictObject({
  communityId: z.string(),
  projectDtag: z.string(),
  repositoryIdentity: z.string(),
  codexThreadId: z.string(),
  executionMode: z.enum(CODE_EXECUTION_MODES),
  executionRoot: z.string(),
  baseRef: z.string(),
  worktreeId: z.string().nullable(),
});

export const CodeThreadPreparationSchema = z
  .strictObject({
    preparationId: z.string(),
    communityId: z.string(),
    projectDtag: z.string(),
    repositoryIdentity: z.string(),
    executionMode: z.enum(CODE_EXECUTION_MODES),
    executionRoot: z.string(),
    baseRef: z.string(),
    worktreeId: z.string().nullable(),
    operation: z.enum(CODE_THREAD_PREPARATION_OPERATIONS),
    sourceThreadId: z.string().min(1).nullable(),
    state: z.enum(CODE_THREAD_PREPARATION_STATES),
  })
  .superRefine((preparation, context) => {
    if (
      preparation.operation === "start" &&
      preparation.sourceThreadId !== null
    ) {
      context.addIssue({
        code: "custom",
        message: "Start preparations cannot carry a fork source",
        path: ["sourceThreadId"],
      });
    }
    if (
      preparation.operation === "fork" &&
      (preparation.sourceThreadId === null ||
        preparation.executionMode !== "worktree" ||
        preparation.worktreeId === null)
    ) {
      context.addIssue({
        code: "custom",
        message:
          "Fork preparations require an exact source and managed destination",
      });
    }
  });

export const CodeTurnSnapshotSchema = z.strictObject({
  id: z.string(),
  status: z.string(),
  items: z.array(JsonValueSchema),
  error: JsonValueSchema.nullable(),
});

export const CodeThreadSummarySchema = z.strictObject({
  id: z.string(),
  sessionId: z.string().nullable(),
  forkedFromId: z.string().nullable(),
  parentThreadId: z.string().nullable(),
  preview: z.string().nullable(),
  ephemeral: z.boolean(),
  modelProvider: z.string().nullable(),
  createdAt: z.number().int().nonnegative().nullable(),
  updatedAt: z.number().int().nonnegative().nullable(),
  cwd: z.string().nullable(),
  name: z.string().nullable(),
  status: JsonValueSchema.nullable(),
  turns: z.array(CodeTurnSnapshotSchema),
});

export const CodeBoundThreadSummarySchema = z.strictObject({
  binding: CodeThreadBindingSchema,
  lifecycle: z.enum(CODE_THREAD_LIFECYCLE_STATES),
  thread: CodeThreadSummarySchema.nullable(),
  unavailable: z.string().nullable(),
});

export const CodeThreadLifecycleMutationResultSchema = z.strictObject({
  binding: CodeThreadBindingSchema,
  lifecycle: z.enum(CODE_THREAD_LIFECYCLE_STATES),
  thread: CodeThreadSummarySchema.nullable(),
});

export const CodeBoundThreadOpenResultSchema = z.strictObject({
  binding: CodeThreadBindingSchema,
  thread: CodeThreadSummarySchema,
  instructionSources: z.array(z.string()),
  model: CodeModelIdentifierSchema,
  reasoningEffort: CodeModelIdentifierSchema.nullable(),
});

export const CodeThreadsPageSchema = z.strictObject({
  data: z.array(CodeBoundThreadSummarySchema),
  nextCursor: z.string().nullable(),
  backwardsCursor: z.string().nullable(),
});

export const CodeTurnSummarySchema = z.strictObject({
  id: z.string(),
  status: z.string(),
});

export const CodeThreadStartErrorSchema = z.strictObject({
  code: z.string(),
  message: z.string(),
  preparationId: z.string().nullable(),
  threadId: z.string().nullable(),
  executionRoot: z.string().nullable(),
});

const APPROVAL_KIND_BY_EVENT = {
  "item/commandExecution/requestApproval": "commandExecution",
  "item/fileChange/requestApproval": "fileChange",
  "item/permissions/requestApproval": "permissions",
} as const;

const CodePermissionSpecialPathDisplaySchema = z.discriminatedUnion("kind", [
  z.strictObject({ kind: z.literal("root") }),
  z.strictObject({ kind: z.literal("minimal") }),
  z.strictObject({
    kind: z.literal("project_roots"),
    subpath: z.string().nullable(),
  }),
  z.strictObject({ kind: z.literal("tmpdir") }),
  z.strictObject({ kind: z.literal("slash_tmp") }),
  z.strictObject({
    kind: z.literal("unknown"),
    path: z.string(),
    subpath: z.string().nullable(),
  }),
]);

const CodePermissionPathDisplaySchema = z.discriminatedUnion("type", [
  z.strictObject({ type: z.literal("path"), path: z.string() }),
  z.strictObject({ type: z.literal("globPattern"), pattern: z.string() }),
  z.strictObject({
    type: z.literal("special"),
    value: CodePermissionSpecialPathDisplaySchema,
  }),
]);

export const CodePermissionDisplaySchema = z.strictObject({
  grantable: z.boolean(),
  network: z.strictObject({ enabled: z.boolean().nullable() }).nullable(),
  fileSystem: z
    .strictObject({
      entries: z
        .array(
          z.strictObject({
            access: z.enum(["read", "write", "deny"]),
            path: CodePermissionPathDisplaySchema,
          }),
        )
        .nullable(),
      globScanMaxDepth: CodeSafeUnsignedIntegerSchema.nullable(),
      read: z.array(z.string()).nullable(),
      write: z.array(z.string()).nullable(),
    })
    .nullable(),
});

export const CodeWorkspaceEventSchema = z
  .strictObject({
    scope: CodeThreadBindingScopeSchema,
    runtimeGeneration: CodeSafeUnsignedIntegerSchema,
    sequence: CodeSafeUnsignedIntegerSchema,
    threadId: z.string().nullable(),
    turnId: z.string().nullable(),
    itemId: z.string().nullable(),
    kind: z.enum(CODE_WORKSPACE_EVENT_KINDS),
    payload: JsonValueSchema,
  })
  .superRefine((event, context) => {
    const approvalKind =
      APPROVAL_KIND_BY_EVENT[event.kind as keyof typeof APPROVAL_KIND_BY_EVENT];
    if (approvalKind === undefined) return;

    if (
      event.threadId === null ||
      event.turnId === null ||
      event.itemId === null ||
      typeof event.payload !== "object" ||
      event.payload === null ||
      Array.isArray(event.payload)
    ) {
      context.addIssue({
        code: "custom",
        message:
          "Approval events require thread, turn, item, and object payloads",
      });
      return;
    }

    const { requestId, request } = event.payload;
    const requestIdValid =
      typeof requestId === "string" ||
      (typeof requestId === "number" &&
        Number.isSafeInteger(requestId) &&
        requestId >= 0);
    if (
      !requestIdValid ||
      event.payload.approvalKind !== approvalKind ||
      typeof request !== "object" ||
      request === null ||
      Array.isArray(request) ||
      request.threadId !== event.threadId ||
      request.turnId !== event.turnId ||
      request.itemId !== event.itemId
    ) {
      context.addIssue({
        code: "custom",
        message: "Approval payload identity must match its event envelope",
      });
      return;
    }
    if (
      approvalKind === "permissions" &&
      ("permissions" in request ||
        !CodePermissionDisplaySchema.safeParse(request.permissionDisplay)
          .success)
    ) {
      context.addIssue({
        code: "custom",
        message: "Permission approvals require display-only permission details",
      });
    }
  });

export const CodeActiveTurnCheckpointSchema = z.strictObject({
  threadId: z.string().min(1),
  turnId: z.string().min(1),
  status: z.string().min(1),
  startedSequence: CodeSafeUnsignedIntegerSchema,
});

export const CodeApprovalCheckpointSchema = z.strictObject({
  event: CodeWorkspaceEventSchema,
  respondable: z.boolean(),
});

export const CodeEventCheckpointSchema = z.strictObject({
  runtimeGeneration: CodeSafeUnsignedIntegerSchema,
  sequenceWatermark: CodeSafeUnsignedIntegerSchema,
  activeTurns: z.array(CodeActiveTurnCheckpointSchema),
  pendingApprovals: z.array(CodeApprovalCheckpointSchema),
});

export const CodeEventBacklogSchema = z
  .strictObject({
    runtimeGeneration: CodeSafeUnsignedIntegerSchema,
    latestSequence: CodeSafeUnsignedIntegerSchema,
    truncated: z.boolean(),
    checkpoint: CodeEventCheckpointSchema.nullable(),
    events: z.array(CodeWorkspaceEventSchema),
  })
  .superRefine((backlog, context) => {
    const checkpoint = backlog.checkpoint;
    if (checkpoint === null) return;
    if (
      checkpoint.runtimeGeneration !== backlog.runtimeGeneration ||
      checkpoint.sequenceWatermark !== backlog.latestSequence
    ) {
      context.addIssue({
        code: "custom",
        message: "Event checkpoint identity must match its replay backlog",
      });
    }
    for (const turn of checkpoint.activeTurns) {
      if (turn.startedSequence > checkpoint.sequenceWatermark) {
        context.addIssue({
          code: "custom",
          message: "Active turn checkpoint exceeds its sequence watermark",
        });
      }
    }
    for (const approval of checkpoint.pendingApprovals) {
      if (
        approval.event.runtimeGeneration !== checkpoint.runtimeGeneration ||
        approval.event.sequence !== checkpoint.sequenceWatermark ||
        !CODE_WORKSPACE_APPROVAL_REQUEST_KINDS.includes(
          approval.event
            .kind as (typeof CODE_WORKSPACE_APPROVAL_REQUEST_KINDS)[number],
        )
      ) {
        context.addIssue({
          code: "custom",
          message:
            "Approval checkpoint must match its watermark and event kind",
        });
      }
    }
  });

export const CodeRuntimeEventsInputSchema = z.union([
  z.strictObject({
    scope: CodeThreadBindingScopeSchema,
    runtimeGeneration: z.null(),
    afterSequence: z.null(),
  }),
  z.strictObject({
    scope: CodeThreadBindingScopeSchema,
    runtimeGeneration: CodeSafeUnsignedIntegerSchema,
    afterSequence: CodeSafeUnsignedIntegerSchema.nullable(),
  }),
]);

const CodeTerminalDimensionSchema = z.number().int().min(1).max(1_000);
const CodeTerminalByteSchema = z.number().int().min(0).max(255);
const CodeTerminalBytesSchema = z.array(CodeTerminalByteSchema).max(64 * 1024);
const CodeTerminalSessionIdSchema = z
  .string()
  .uuid()
  .regex(/^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/);
const CodeTerminalIdentityShape = {
  scope: CodeThreadBindingScopeSchema,
  threadId: z.string().min(1),
  sessionId: CodeTerminalSessionIdSchema,
};
const CodeTerminalEventIdentityShape = {
  ...CodeTerminalIdentityShape,
  sequence: CodeSafeUnsignedIntegerSchema.min(1),
};

export const CodeTerminalOpenInputSchema = z.strictObject({
  scope: CodeThreadBindingScopeSchema,
  threadId: z.string().min(1),
  cols: CodeTerminalDimensionSchema,
  rows: CodeTerminalDimensionSchema,
});

export const CodeTerminalSessionSchema = z.strictObject({
  ...CodeTerminalIdentityShape,
  cols: CodeTerminalDimensionSchema,
  rows: CodeTerminalDimensionSchema,
});

export const CodeTerminalResizeInputSchema = CodeTerminalSessionSchema;

export const CodeTerminalStdinInputSchema = z.strictObject({
  ...CodeTerminalIdentityShape,
  data: CodeTerminalBytesSchema,
});

export const CodeTerminalTerminateInputSchema = z.strictObject({
  ...CodeTerminalIdentityShape,
});

export const CodeTerminalOutputEventSchema = z.strictObject({
  type: z.literal("output"),
  ...CodeTerminalEventIdentityShape,
  data: CodeTerminalBytesSchema,
});

export const CodeTerminalExitEventSchema = z.strictObject({
  type: z.literal("exit"),
  ...CodeTerminalEventIdentityShape,
  exitCode: z.number().int().nonnegative().max(4_294_967_295),
  signal: z.string().nullable(),
});

export const CodeTerminalEventSchema = z.discriminatedUnion("type", [
  CodeTerminalOutputEventSchema,
  CodeTerminalExitEventSchema,
]);

export const CodeWorktreePrepareInputSchema = z.strictObject({
  scope: CodeThreadBindingScopeSchema,
  repositoryRoot: z.string(),
  baseRef: z.string(),
  executionMode: z.enum(CODE_EXECUTION_MODES),
});

export const CodeThreadPreparationListInputSchema = z.strictObject({
  scope: CodeThreadBindingScopeSchema,
});

export const CodeThreadListInputSchema = z.strictObject({
  scope: CodeThreadBindingScopeSchema,
});

export const CodeWorktreesListInputSchema = z.strictObject({
  scope: CodeThreadBindingScopeSchema,
});

export const CodeWorktreeRemoveInputSchema = z.strictObject({
  scope: CodeThreadBindingScopeSchema,
  threadId: z.string().min(1),
});

const CodeGitObjectIdSchema = z
  .string()
  .regex(/^(?:[0-9a-f]{40}|[0-9a-f]{64})$/);

const CodeCanonicalUuidSchema = z
  .string()
  .regex(/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/);

const CodeCanonicalUuidV4Schema = z
  .string()
  .regex(
    /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
  );

function isSafeDirectLocalBranchRef(value: string): boolean {
  const prefix = "refs/heads/";
  if (
    !value.startsWith(prefix) ||
    new TextEncoder().encode(value).length > 512
  ) {
    return false;
  }
  const branch = value.slice(prefix.length);
  if (
    branch.length === 0 ||
    value.endsWith("/") ||
    value.endsWith(".") ||
    value.includes("..") ||
    value.includes("@{") ||
    value.includes("//")
  ) {
    return false;
  }
  const forbiddenCharacters = new Set(["~", "^", ":", "?", "*", "[", "\\"]);
  if (
    Array.from(value).some(
      (character) =>
        /[\p{Cc}\s]/u.test(character) || forbiddenCharacters.has(character),
    )
  ) {
    return false;
  }
  return branch
    .split("/")
    .every(
      (component) =>
        component.length > 0 &&
        component !== "." &&
        component !== ".." &&
        component !== "@" &&
        !component.startsWith(".") &&
        !component.endsWith(".lock"),
    );
}

const CodeDirectLocalBranchRefSchema = z
  .string()
  .refine(isSafeDirectLocalBranchRef, {
    message: "Removal receipt must name a safe direct local branch ref",
  });

export const CodeWorktreeRemovalReceiptSchema = z
  .strictObject({
    removalId: CodeCanonicalUuidV4Schema,
    scope: CodeThreadBindingScopeSchema,
    threadId: z.string().min(1),
    worktreeId: CodeCanonicalUuidSchema,
    headCommit: CodeGitObjectIdSchema,
    mergedIntoRef: CodeDirectLocalBranchRefSchema,
    mergedIntoCommit: CodeGitObjectIdSchema,
    transcriptDisposition: z.literal("preserved"),
    executionDisposition: z.literal("removed"),
  })
  .superRefine((receipt, context) => {
    if (receipt.headCommit.length !== receipt.mergedIntoCommit.length) {
      context.addIssue({
        code: "custom",
        message: "Removal receipt cannot mix Git object-id formats",
        path: ["mergedIntoCommit"],
      });
    }
  });

export const CodeThreadForkInputSchema = z.strictObject({
  scope: CodeThreadBindingScopeSchema,
  threadId: z.string().min(1),
});

export const CodeThreadLifecycleMutationInputSchema = z.strictObject({
  scope: CodeThreadBindingScopeSchema,
  threadId: z.string().min(1),
});

const CodeThreadNameSchema = z
  .string()
  .min(1)
  .refine((name) => name.trim() === name, {
    message: "Thread name must not have leading or trailing whitespace",
  })
  .refine((name) => Array.from(name).length <= 128, {
    message: "Thread name must contain at most 128 Unicode scalar values",
  })
  .refine((name) => new TextEncoder().encode(name).byteLength <= 512, {
    message: "Thread name must contain at most 512 UTF-8 bytes",
  })
  .refine((name) => !/\p{Cc}/u.test(name), {
    message: "Thread name must not contain control characters",
  });

export const CodeThreadRenameInputSchema = z.strictObject({
  scope: CodeThreadBindingScopeSchema,
  threadId: z.string().min(1),
  name: CodeThreadNameSchema,
});

export const CodeThreadChangesInputSchema = z.strictObject({
  scope: CodeThreadBindingScopeSchema,
  threadId: z.string(),
});

export const CodeThreadChangedFileSchema = z.strictObject({
  path: z.string().min(1),
  status: z.enum(CODE_THREAD_CHANGE_STATUSES),
  binary: z.boolean(),
  additions: CodeSafeUnsignedIntegerSchema,
  deletions: CodeSafeUnsignedIntegerSchema,
  patch: z.string(),
  truncated: z.boolean(),
});

export const CodeThreadChangesSchema = z
  .strictObject({
    files: z.array(CodeThreadChangedFileSchema),
    additions: CodeSafeUnsignedIntegerSchema,
    deletions: CodeSafeUnsignedIntegerSchema,
    commitBody: z.string().nullable(),
    totalFiles: CodeSafeUnsignedIntegerSchema,
    filesTruncated: z.boolean(),
  })
  .superRefine((changes, context) => {
    const paths = new Set<string>();
    let additions = 0;
    let deletions = 0;
    for (const [index, file] of changes.files.entries()) {
      if (paths.has(file.path)) {
        context.addIssue({
          code: "custom",
          message: "Changed file paths must be unique",
          path: ["files", index, "path"],
        });
      }
      paths.add(file.path);
      if (file.binary && (file.additions !== 0 || file.deletions !== 0)) {
        context.addIssue({
          code: "custom",
          message: "Binary changed files cannot report text line totals",
          path: ["files", index],
        });
      }
      additions += file.additions;
      deletions += file.deletions;
    }
    if (
      !Number.isSafeInteger(additions) ||
      !Number.isSafeInteger(deletions) ||
      changes.additions !== additions ||
      changes.deletions !== deletions
    ) {
      context.addIssue({
        code: "custom",
        message: "Change totals must equal the returned file subset",
      });
    }
    if (changes.files.length > changes.totalFiles) {
      context.addIssue({
        code: "custom",
        message: "Changed file results cannot exceed the reported total",
        path: ["files"],
      });
    }
    if (changes.filesTruncated !== changes.files.length < changes.totalFiles) {
      context.addIssue({
        code: "custom",
        message:
          "Changed file truncation must exactly match the reported file total",
        path: ["filesTruncated"],
      });
    }
  });

export const CodeThreadStartInputSchema = z.strictObject({
  scope: CodeThreadBindingScopeSchema,
  preparationId: z.string(),
  model: z.string().nullable(),
});

export const CodeThreadBindingRecoverInputSchema = z.strictObject({
  scope: CodeThreadBindingScopeSchema,
  preparationId: z.string(),
  model: z.string().nullable(),
});

export const CodeThreadResumeInputSchema = z.strictObject({
  scope: CodeThreadBindingScopeSchema,
  threadId: z.string(),
  model: z.string().nullable(),
});

export const CodeTurnStartInputSchema = z.strictObject({
  scope: CodeThreadBindingScopeSchema,
  threadId: z.string(),
  prompt: z.string(),
  model: z.string().nullable(),
  effort: z.string().nullable(),
});

export const CodeTurnSteerInputSchema = z.strictObject({
  scope: CodeThreadBindingScopeSchema,
  threadId: z.string(),
  expectedTurnId: z.string(),
  prompt: z.string(),
});

export const CodeTurnInterruptInputSchema = z.strictObject({
  scope: CodeThreadBindingScopeSchema,
  threadId: z.string(),
  turnId: z.string(),
});

export const CodeRequestIdSchema = z.union([
  z.string(),
  CodeSafeUnsignedIntegerSchema,
]);

export const CodeApprovalResponseSchema = z.discriminatedUnion("type", [
  z.strictObject({
    type: z.literal("decision"),
    decision: z.enum(CODE_APPROVAL_DECISIONS),
  }),
  z.strictObject({
    type: z.literal("permissions"),
    intent: z.enum(CODE_PERMISSION_INTENTS),
    scope: z.enum(CODE_PERMISSION_SCOPES),
  }),
]);

export const CodeApprovalResponseInputSchema = z.strictObject({
  runtimeGeneration: CodeSafeUnsignedIntegerSchema,
  requestId: CodeRequestIdSchema,
  scope: CodeThreadBindingScopeSchema,
  threadId: z.string(),
  turnId: z.string(),
  response: CodeApprovalResponseSchema,
});

export const CodeUnitResponseSchema = z.null();

/** Schemas consumed by the cross-language fixture compatibility test. */
export const codeWorkspaceOutputSchemas = {
  runtimeProbe: CodeRuntimeProbeSchema,
  runtimeStatus: CodeRuntimeStatusSchema,
  modelCatalog: CodeModelsCatalogSchema,
  modelSelection: CodeModelSelectionSchema,
  repositoryDescriptor: CodeRepositoryDescriptorSchema,
  binding: CodeThreadBindingSchema,
  preparationPublicBaseline: CodeThreadPreparationSchema,
  preparedWorktree: CodePreparedWorktreeSchema,
  worktreeStatus: CodeWorktreeStatusSchema,
  worktreeInventory: z.array(CodeWorktreeInventoryRowSchema),
  worktreeRemovalReceipt: CodeWorktreeRemovalReceiptSchema,
  preparationList: z.array(CodeThreadPreparationSchema),
  threadSummary: CodeThreadSummarySchema,
  threadLifecycleMutation: CodeThreadLifecycleMutationResultSchema,
  threadsPage: CodeThreadsPageSchema,
  threadChanges: CodeThreadChangesSchema,
  boundThreadOpen: CodeBoundThreadOpenResultSchema,
  event: CodeWorkspaceEventSchema,
  eventWithoutIds: CodeWorkspaceEventSchema,
  eventBacklog: CodeEventBacklogSchema,
  terminalSession: CodeTerminalSessionSchema,
  terminalOutputEvent: CodeTerminalOutputEventSchema,
  terminalExitEvent: CodeTerminalExitEventSchema,
  turnSummary: CodeTurnSummarySchema,
  threadStartError: CodeThreadStartErrorSchema,
  unitResponse: CodeUnitResponseSchema,
} as const;
