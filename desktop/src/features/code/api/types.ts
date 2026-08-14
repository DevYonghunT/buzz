/** JSON values accepted at the frozen SchoolX Code Tauri boundary. */
export type JsonPrimitive = string | number | boolean | null;

/** A recursively JSON-serializable value. */
export type JsonValue =
  | JsonPrimitive
  | JsonValue[]
  | { [key: string]: JsonValue };

/** A JSON object used for normalized payloads and permission subsets. */
export type JsonObject = { [key: string]: JsonValue };

/** Exact community/project/repository coordinate enforced by native bindings. */
export type CodeThreadBindingScope = {
  communityId: string;
  projectDtag: string;
  repositoryIdentity: string;
};

/** Compare every field in the native binding scope. */
export function codeScopesEqual(
  left: CodeThreadBindingScope,
  right: CodeThreadBindingScope,
): boolean {
  return (
    left.communityId === right.communityId &&
    left.projectDtag === right.projectDtag &&
    left.repositoryIdentity === right.repositoryIdentity
  );
}

export const CODE_EXECUTION_MODES = ["worktree", "local"] as const;
export type CodeExecutionMode = (typeof CODE_EXECUTION_MODES)[number];

export const CODE_THREAD_PREPARATION_STATES = ["prepared", "starting"] as const;
export type CodeThreadPreparationState =
  (typeof CODE_THREAD_PREPARATION_STATES)[number];

export const CODE_RUNTIME_PHASES = [
  "notInstalled",
  "stopped",
  "starting",
  "initializing",
  "ready",
  "stopping",
  "failed",
] as const;
export type CodeRuntimePhase = (typeof CODE_RUNTIME_PHASES)[number];

export const CODE_APPROVAL_DECISIONS = [
  "accept",
  "acceptForSession",
  "decline",
  "cancel",
] as const;
export type CodeApprovalDecision = (typeof CODE_APPROVAL_DECISIONS)[number];

export const CODE_PERMISSION_SCOPES = ["turn", "session"] as const;
export type CodePermissionScope = (typeof CODE_PERMISSION_SCOPES)[number];

export const CODE_APPROVAL_RESPONSE_TYPES = [
  "decision",
  "permissions",
] as const;

/** String and numeric JSON-RPC ids remain distinct approval identities. */
export type CodeRequestId = string | number;

/** Normalized response accepted for command and file-change approvals. */
export type CodeApprovalDecisionResponse = {
  type: "decision";
  decision: CodeApprovalDecision;
};

/** Explicit subset accepted for a permission approval. */
export type CodeApprovalPermissionsResponse = {
  type: "permissions";
  permissions: JsonObject;
  scope: CodePermissionScope;
  strictAutoReview: boolean;
};

/** Tagged response forwarded to the native approval gate. */
export type CodeApprovalResponse =
  | CodeApprovalDecisionResponse
  | CodeApprovalPermissionsResponse;

/** Input whose full identity must match one pending native approval. */
export type CodeApprovalResponseInput = {
  runtimeGeneration: number;
  requestId: CodeRequestId;
  scope: CodeThreadBindingScope;
  threadId: string;
  turnId: string;
  response: CodeApprovalResponse;
};

/** Result of locating the supported Codex executable. */
export type CodeRuntimeProbe = {
  available: boolean;
  executable: string | null;
  version: string | null;
  error: string | null;
};

/** Desktop-wide Codex app-server lifecycle snapshot. */
export type CodeRuntimeStatus = {
  phase: CodeRuntimePhase;
  generation: number;
  executable: string | null;
  version: string | null;
  pid: number | null;
  userAgent: string | null;
  codexHome: string | null;
  platformFamily: string | null;
  platformOs: string | null;
  queuedNotifications: number;
  lastError: string | null;
};

/** Persistable native execution-root descriptor. */
export type CodeWorktreeDescriptor = {
  executionMode: CodeExecutionMode;
  repositoryIdentity: string;
  executionRoot: string;
  baseRef: string;
  worktreeId: string | null;
};

/** Canonical identity of the selected local Git repository. */
export type CodeRepositoryDescriptor = {
  repositoryRoot: string;
  gitCommonDir: string;
  repositoryIdentity: string;
};

/** Read-only Git selection used to establish the native repository scope. */
export type CodeRepositoryInspectInput = {
  repositoryRoot: string;
  baseRef: string;
};

/** Git state returned after native preparation. */
export type CodeWorktreePrepareResult = {
  repository: CodeRepositoryDescriptor;
  descriptor: CodeWorktreeDescriptor;
  headCommit: string;
  branch: string | null;
  dirty: boolean;
};

/** Durable preparation issued before a Codex thread is started. */
export type CodePreparedWorktree = {
  preparationId: string;
  scope: CodeThreadBindingScope;
  worktree: CodeWorktreePrepareResult;
};

/** Revalidated status of one persisted execution root. */
export type CodeWorktreeStatus = {
  descriptor: CodeWorktreeDescriptor;
  headCommit: string;
  branch: string | null;
  dirty: boolean;
};

/** Durable native binding between one Codex thread and execution root. */
export type CodeThreadBinding = {
  communityId: string;
  projectDtag: string;
  repositoryIdentity: string;
  codexThreadId: string;
  executionMode: CodeExecutionMode;
  executionRoot: string;
  baseRef: string;
  worktreeId: string | null;
};

/** Public unfinished preparation; private recovery baselines never cross Tauri. */
export type CodeThreadPreparation = {
  preparationId: string;
  communityId: string;
  projectDtag: string;
  repositoryIdentity: string;
  executionMode: CodeExecutionMode;
  executionRoot: string;
  baseRef: string;
  worktreeId: string | null;
  state: CodeThreadPreparationState;
};

/** Persisted turn included while a native thread is hydrated. */
export type CodeTurnSnapshot = {
  id: string;
  status: string;
  items: JsonValue[];
  error: JsonValue | null;
};

/** Narrow, redacted Codex thread metadata exposed by native code. */
export type CodeThreadSummary = {
  id: string;
  sessionId: string | null;
  forkedFromId: string | null;
  parentThreadId: string | null;
  preview: string | null;
  ephemeral: boolean;
  modelProvider: string | null;
  createdAt: number | null;
  updatedAt: number | null;
  cwd: string | null;
  name: string | null;
  status: JsonValue | null;
  turns: CodeTurnSnapshot[];
};

/** One durable binding with optional live app-server metadata. */
export type CodeBoundThreadSummary = {
  binding: CodeThreadBinding;
  thread: CodeThreadSummary | null;
  unavailable: string | null;
};

/** Result shared by native thread start, recovery, and resume. */
export type CodeBoundThreadOpenResult = {
  binding: CodeThreadBinding;
  thread: CodeThreadSummary;
  instructionSources: string[];
};

/** Project-scoped durable thread listing. */
export type CodeThreadsPage = {
  data: CodeBoundThreadSummary[];
  nextCursor: string | null;
  backwardsCursor: string | null;
};

/** Narrow result returned when a turn starts or accepts steering. */
export type CodeTurnSummary = {
  id: string;
  status: string;
};

/** Structured native rejection from the uncertain thread-start boundary. */
export type CodeThreadStartError = {
  code: string;
  message: string;
  preparationId: string | null;
  threadId: string | null;
  executionRoot: string | null;
};

export const CODE_WORKSPACE_NOTIFICATION_KINDS = [
  "error",
  "warning",
  "configWarning",
  "thread/started",
  "thread/status/changed",
  "thread/closed",
  "turn/started",
  "turn/completed",
  "turn/diff/updated",
  "turn/plan/updated",
  "item/started",
  "item/completed",
  "item/agentMessage/delta",
  "item/plan/delta",
  "item/reasoning/summaryTextDelta",
  "item/reasoning/summaryPartAdded",
  "item/reasoning/textDelta",
  "item/commandExecution/outputDelta",
  "item/commandExecution/terminalInteraction",
  "item/fileChange/patchUpdated",
  "serverRequest/resolved",
] as const;

export const CODE_WORKSPACE_APPROVAL_REQUEST_KINDS = [
  "item/commandExecution/requestApproval",
  "item/fileChange/requestApproval",
  "item/permissions/requestApproval",
] as const;

export const CODE_WORKSPACE_EVENT_KINDS = [
  ...CODE_WORKSPACE_NOTIFICATION_KINDS,
  ...CODE_WORKSPACE_APPROVAL_REQUEST_KINDS,
] as const;
export type CodeWorkspaceEventKind =
  (typeof CODE_WORKSPACE_EVENT_KINDS)[number];

/** Scoped, normalized event emitted by the native app-server bridge. */
export type CodeWorkspaceEvent = {
  scope: CodeThreadBindingScope;
  runtimeGeneration: number;
  sequence: number;
  threadId: string | null;
  turnId: string | null;
  itemId: string | null;
  kind: CodeWorkspaceEventKind;
  payload: JsonValue;
};

/** Native replay result using the desktop-wide runtime sequence cursor. */
export type CodeEventBacklog = {
  runtimeGeneration: number;
  latestSequence: number;
  truncated: boolean;
  events: CodeWorkspaceEvent[];
};

/** Arguments passed directly to the replay command. */
export type CodeRuntimeEventsInput =
  | {
      scope: CodeThreadBindingScope;
      runtimeGeneration: null;
      afterSequence: null;
    }
  | {
      scope: CodeThreadBindingScope;
      runtimeGeneration: number;
      afterSequence: number | null;
    };

/** Scope plus Git selection used to prepare an execution root. */
export type CodeWorktreePrepareInput = {
  scope: CodeThreadBindingScope;
  repositoryRoot: string;
  baseRef: string;
  executionMode: CodeExecutionMode;
};

export type CodeThreadPreparationListInput = {
  scope: CodeThreadBindingScope;
};

export type CodeThreadListInput = {
  scope: CodeThreadBindingScope;
};

export type CodeThreadStartInput = {
  scope: CodeThreadBindingScope;
  preparationId: string;
  model: string | null;
};

export type CodeThreadBindingRecoverInput = {
  scope: CodeThreadBindingScope;
  preparationId: string;
  model: string | null;
};

export type CodeThreadResumeInput = {
  scope: CodeThreadBindingScope;
  threadId: string;
  model: string | null;
};

export type CodeTurnStartInput = {
  scope: CodeThreadBindingScope;
  threadId: string;
  prompt: string;
  model: string | null;
  effort: string | null;
};

export type CodeTurnSteerInput = {
  scope: CodeThreadBindingScope;
  threadId: string;
  expectedTurnId: string;
  prompt: string;
};

export type CodeTurnInterruptInput = {
  scope: CodeThreadBindingScope;
  threadId: string;
  turnId: string;
};
