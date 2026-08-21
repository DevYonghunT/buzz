/** JSON values accepted at the frozen SchoolX Code Tauri boundary. */
export type JsonPrimitive = string | number | boolean | null;

/** A recursively JSON-serializable value. */
export type JsonValue =
  | JsonPrimitive
  | JsonValue[]
  | { [key: string]: JsonValue };

/** A JSON object used for normalized payloads. */
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

export const CODE_THREAD_PREPARATION_OPERATIONS = ["start", "fork"] as const;
export type CodeThreadPreparationOperation =
  (typeof CODE_THREAD_PREPARATION_OPERATIONS)[number];

export const CODE_THREAD_LIFECYCLE_STATES = [
  "active",
  "archiving",
  "archived",
  "unarchiving",
  "unknown",
] as const;
export type CodeThreadLifecycleState =
  (typeof CODE_THREAD_LIFECYCLE_STATES)[number];

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

export const CODE_PERMISSION_INTENTS = ["grant", "decline"] as const;
export type CodePermissionIntent = (typeof CODE_PERMISSION_INTENTS)[number];

export const CODE_APPROVAL_RESPONSE_TYPES = [
  "decision",
  "permissions",
] as const;

export type CodePermissionAccess = "read" | "write" | "deny";

export type CodePermissionSpecialPathDisplay =
  | { kind: "root" }
  | { kind: "minimal" }
  | { kind: "project_roots"; subpath: string | null }
  | { kind: "tmpdir" }
  | { kind: "slash_tmp" }
  | { kind: "unknown"; path: string; subpath: string | null };

export type CodePermissionPathDisplay =
  | { type: "path"; path: string }
  | { type: "globPattern"; pattern: string }
  | { type: "special"; value: CodePermissionSpecialPathDisplay };

export type CodePermissionFileSystemEntryDisplay = {
  access: CodePermissionAccess;
  path: CodePermissionPathDisplay;
};

/** Display-only permission details; never accepted as response authority. */
export type CodePermissionDisplay = {
  grantable: boolean;
  network: { enabled: boolean | null } | null;
  fileSystem: {
    entries: CodePermissionFileSystemEntryDisplay[] | null;
    globScanMaxDepth: number | null;
    read: string[] | null;
    write: string[] | null;
  } | null;
};

/** String and numeric JSON-RPC ids remain distinct approval identities. */
export type CodeRequestId = string | number;

/** Normalized response accepted for command and file-change approvals. */
export type CodeApprovalDecisionResponse = {
  type: "decision";
  decision: CodeApprovalDecision;
};

/** Opaque permission intent; native code owns the raw requested authority. */
export type CodeApprovalPermissionsResponse = {
  type: "permissions";
  intent: CodePermissionIntent;
  scope: CodePermissionScope;
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

/** One reasoning-effort choice advertised for a normalized Codex model. */
export type CodeReasoningEffortOption = {
  reasoningEffort: string;
  description: string;
};

/** One visible model preset exposed by the active Codex runtime. */
export type CodeModelOption = {
  id: string;
  model: string;
  displayName: string;
  description: string;
  isDefault: boolean;
  defaultReasoningEffort: string;
  supportedReasoningEfforts: CodeReasoningEffortOption[];
};

/** Persisted user choice. `reasoningEffort` maps to turn/start `effort`. */
export type CodeModelSelection = {
  model: string;
  reasoningEffort: string;
};

/** Generation-bound model catalog and the last validated SchoolX choice. */
export type CodeModelsCatalog = {
  runtimeGeneration: number;
  models: CodeModelOption[];
  recentSelection: CodeModelSelection | null;
};

/** Persistable native execution-root descriptor. */
export type CodeWorktreeDescriptor = {
  executionMode: CodeExecutionMode;
  repositoryIdentity: string;
  executionRoot: string;
  baseRef: string;
  worktreeId: string | null;
};

/** Closed native reasons that keep a managed worktree preserved. */
export const CODE_WORKTREE_INVENTORY_BLOCKERS = [
  "activeBinding",
  "lifecycleUnsettled",
  "unfinishedPreparation",
  "localCheckout",
  "unavailableRoot",
  "dirtyRoot",
  "branchAttached",
  "headDrift",
  "mergeProofUnavailable",
] as const;
export type CodeWorktreeInventoryBlocker =
  (typeof CODE_WORKTREE_INVENTORY_BLOCKERS)[number];

/** Managed-only descriptor accepted in the read-only inventory. */
export type CodeManagedWorktreeDescriptor = CodeWorktreeDescriptor & {
  executionMode: "worktree";
  worktreeId: string;
};

/** Durable native record that authorizes one inventory row. */
export type CodeWorktreeInventoryAuthority =
  | {
      type: "binding";
      threadId: string;
      lifecycle: CodeThreadLifecycleState;
    }
  | {
      type: "preparation";
      preparationId: string;
      operation: CodeThreadPreparationOperation;
      state: CodeThreadPreparationState;
      sourceThreadId: string | null;
    };

/** Row-local, read-only inspection result; one failed root cannot hide peers. */
export type CodeWorktreeInspection =
  | {
      status: "available";
      headCommit: string;
      branch: string | null;
      dirty: boolean;
    }
  | { status: "unavailable"; error: string };

/** Native-derived preservation projection for one managed execution root. */
export type CodeWorktreeInventoryRow = {
  scope: CodeThreadBindingScope;
  authority: CodeWorktreeInventoryAuthority;
  descriptor: CodeManagedWorktreeDescriptor;
  inspection: CodeWorktreeInspection;
  preserved: true;
  canRemove: boolean;
  blockers: CodeWorktreeInventoryBlocker[];
};

/** Exact public coordinate accepted by native managed-worktree removal. */
export type CodeWorktreeRemoveInput = {
  scope: CodeThreadBindingScope;
  threadId: string;
};

/** Native-derived receipt for one durable, idempotent worktree removal. */
export type CodeWorktreeRemovalReceipt = {
  removalId: string;
  scope: CodeThreadBindingScope;
  threadId: string;
  worktreeId: string;
  headCommit: string;
  mergedIntoRef: string;
  mergedIntoCommit: string;
  transcriptDisposition: "preserved";
  executionDisposition: "removed";
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
  operation: CodeThreadPreparationOperation;
  sourceThreadId: string | null;
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
  lifecycle: CodeThreadLifecycleState;
  thread: CodeThreadSummary | null;
  unavailable: string | null;
};

/** Stable public projection returned after an exact lifecycle mutation. */
export type CodeThreadLifecycleMutationResult = {
  binding: CodeThreadBinding;
  lifecycle: CodeThreadLifecycleState;
  thread: CodeThreadSummary | null;
};

/** Result shared by native thread start, recovery, and resume. */
export type CodeBoundThreadOpenResult = {
  binding: CodeThreadBinding;
  thread: CodeThreadSummary;
  instructionSources: string[];
  model: string;
  reasoningEffort: string | null;
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
  "thread/archived",
  "thread/unarchived",
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

export type CodeActiveTurnCheckpoint = {
  threadId: string;
  turnId: string;
  status: string;
  startedSequence: number;
};

export type CodeApprovalCheckpoint = {
  event: CodeWorkspaceEvent;
  /** False only while native has reserved an in-flight response write. */
  respondable: boolean;
};

/** Authoritative transient runtime state at one exact event watermark. */
export type CodeEventCheckpoint = {
  runtimeGeneration: number;
  sequenceWatermark: number;
  activeTurns: CodeActiveTurnCheckpoint[];
  pendingApprovals: CodeApprovalCheckpoint[];
};

/** Native replay result using the desktop-wide runtime sequence cursor. */
export type CodeEventBacklog = {
  runtimeGeneration: number;
  latestSequence: number;
  truncated: boolean;
  checkpoint: CodeEventCheckpoint | null;
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

/** Initial PTY dimensions for a shell owned by one exact bound thread. */
export type CodeTerminalOpenInput = {
  scope: CodeThreadBindingScope;
  threadId: string;
  cols: number;
  rows: number;
};

/** Native-owned PTY identity returned after the shell has started. */
export type CodeTerminalSession = {
  scope: CodeThreadBindingScope;
  threadId: string;
  sessionId: string;
  cols: number;
  rows: number;
};

/** Exact native PTY whose dimensions should change. */
export type CodeTerminalResizeInput = CodeTerminalSession;

/** Bounded bytes written to one exact native PTY. */
export type CodeTerminalStdinInput = {
  scope: CodeThreadBindingScope;
  threadId: string;
  sessionId: string;
  data: number[];
};

/** Exact native PTY whose shell and descendants should be terminated. */
export type CodeTerminalTerminateInput = {
  scope: CodeThreadBindingScope;
  threadId: string;
  sessionId: string;
};

type CodeTerminalEventIdentity = {
  scope: CodeThreadBindingScope;
  threadId: string;
  sessionId: string;
  /** Monotonic within one native terminal session. */
  sequence: number;
};

/** Arbitrary PTY output bytes; callers must not assume UTF-8 boundaries. */
export type CodeTerminalOutputEvent = CodeTerminalEventIdentity & {
  type: "output";
  data: number[];
};

/** Terminal completion emitted after native process-tree cleanup. */
export type CodeTerminalExitEvent = CodeTerminalEventIdentity & {
  type: "exit";
  exitCode: number;
  signal: string | null;
};

/** Strict channel event union for one native terminal session. */
export type CodeTerminalEvent = CodeTerminalOutputEvent | CodeTerminalExitEvent;

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

/** Exact scope accepted by the read-only managed-worktree inventory. */
export type CodeWorktreesListInput = {
  scope: CodeThreadBindingScope;
};

/** Exact stable managed source whose complete persisted history is forked. */
export type CodeThreadForkInput = {
  scope: CodeThreadBindingScope;
  threadId: string;
};

/** Exact persisted binding whose lifecycle should be changed. */
export type CodeThreadLifecycleMutationInput = {
  scope: CodeThreadBindingScope;
  threadId: string;
};

/** Exact bound thread and strictly bounded title accepted by native rename. */
export type CodeThreadRenameInput = {
  scope: CodeThreadBindingScope;
  threadId: string;
  name: string;
};

/** Exact bound thread whose native execution root should be diffed. */
export type CodeThreadChangesInput = {
  scope: CodeThreadBindingScope;
  threadId: string;
};

export const CODE_THREAD_CHANGE_STATUSES = [
  "added",
  "modified",
  "deleted",
  "typeChanged",
  "unmerged",
  "untracked",
] as const;
export type CodeThreadChangeStatus =
  (typeof CODE_THREAD_CHANGE_STATUSES)[number];

/** One bounded file patch from the selected thread's execution root. */
export type CodeThreadChangedFile = {
  path: string;
  status: CodeThreadChangeStatus;
  binary: boolean;
  additions: number;
  deletions: number;
  patch: string;
  truncated: boolean;
};

/** Current read-only changes relative to the binding's persisted base ref. */
export type CodeThreadChanges = {
  files: CodeThreadChangedFile[];
  additions: number;
  deletions: number;
  commitBody: string | null;
  totalFiles: number;
  filesTruncated: boolean;
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
