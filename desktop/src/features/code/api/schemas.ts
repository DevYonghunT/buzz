import { z } from "zod";

import {
  CODE_APPROVAL_DECISIONS,
  CODE_EXECUTION_MODES,
  CODE_PERMISSION_SCOPES,
  CODE_RUNTIME_PHASES,
  CODE_THREAD_PREPARATION_STATES,
  CODE_WORKSPACE_EVENT_KINDS,
} from "./types";

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

export const CodeThreadPreparationSchema = z.strictObject({
  preparationId: z.string(),
  communityId: z.string(),
  projectDtag: z.string(),
  repositoryIdentity: z.string(),
  executionMode: z.enum(CODE_EXECUTION_MODES),
  executionRoot: z.string(),
  baseRef: z.string(),
  worktreeId: z.string().nullable(),
  state: z.enum(CODE_THREAD_PREPARATION_STATES),
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
  thread: CodeThreadSummarySchema.nullable(),
  unavailable: z.string().nullable(),
});

export const CodeBoundThreadOpenResultSchema = z.strictObject({
  binding: CodeThreadBindingSchema,
  thread: CodeThreadSummarySchema,
  instructionSources: z.array(z.string()),
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
    }
  });

export const CodeEventBacklogSchema = z.strictObject({
  runtimeGeneration: CodeSafeUnsignedIntegerSchema,
  latestSequence: CodeSafeUnsignedIntegerSchema,
  truncated: z.boolean(),
  events: z.array(CodeWorkspaceEventSchema),
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
    permissions: JsonObjectSchema,
    scope: z.enum(CODE_PERMISSION_SCOPES),
    strictAutoReview: z.boolean(),
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
  repositoryDescriptor: CodeRepositoryDescriptorSchema,
  binding: CodeThreadBindingSchema,
  preparationPublicBaseline: CodeThreadPreparationSchema,
  preparedWorktree: CodePreparedWorktreeSchema,
  worktreeStatus: CodeWorktreeStatusSchema,
  preparationList: z.array(CodeThreadPreparationSchema),
  threadSummary: CodeThreadSummarySchema,
  threadsPage: CodeThreadsPageSchema,
  boundThreadOpen: CodeBoundThreadOpenResultSchema,
  event: CodeWorkspaceEventSchema,
  eventWithoutIds: CodeWorkspaceEventSchema,
  eventBacklog: CodeEventBacklogSchema,
  turnSummary: CodeTurnSummarySchema,
  threadStartError: CodeThreadStartErrorSchema,
  unitResponse: CodeUnitResponseSchema,
} as const;
