import type { CodeThreadBindingScope } from "./types";

export type CodeGitOperation = "stage" | "unstage" | "commit";
export type CodeGitChangeStatus =
  | "added"
  | "modified"
  | "deleted"
  | "typeChanged"
  | "unmerged"
  | "untracked";

export type CodeGitStatusInput = {
  scope: CodeThreadBindingScope;
  threadId: string;
};

export type CodeGitIndexMutationInput = CodeGitStatusInput & {
  writeGeneration: number;
  snapshotId: string;
  fileId: string;
};

export type CodeGitCommitInput = CodeGitStatusInput & {
  writeGeneration: number;
  snapshotId: string;
  message: string;
};

export type CodeGitAcknowledgeInput = CodeGitStatusInput & {
  operationId: string;
  writeGeneration: number;
  snapshotId: string;
};

export type CodeGitChangeFile = {
  fileId: string;
  path: string;
  status: CodeGitChangeStatus;
  binary: boolean;
  additions: number;
  deletions: number;
  patch: string;
  truncated: boolean;
};

export type CodeGitChangeSet = {
  files: CodeGitChangeFile[];
  totalFiles: number;
  filesTruncated: boolean;
  additions: number;
  deletions: number;
};

export type CodeGitCapability = { enabled: boolean; reason: string | null };

export type CodeGitIndexMutationReceipt = {
  operationId: string;
  operation: "stage" | "unstage";
  scope: CodeThreadBindingScope;
  threadId: string;
  requestGeneration: number;
  beforeSnapshotId: string;
  fileId: string;
  disposition: "staged" | "unstaged";
};

export type CodeGitCommitReceipt = {
  operationId: string;
  operation: "commit";
  scope: CodeThreadBindingScope;
  threadId: string;
  requestGeneration: number;
  beforeSnapshotId: string;
  previousHead: string;
  commit: string;
  tree: string;
  disposition: "committed";
};

export type CodeGitMutationReceipt =
  | CodeGitIndexMutationReceipt
  | CodeGitCommitReceipt;

export type CodeGitReadyStatus = {
  state: "ready";
  runtimeGeneration: number;
  statusRevision: number;
  writeGeneration: number;
  snapshotSequence: number;
  scope: CodeThreadBindingScope;
  threadId: string;
  snapshotId: string;
  headCommit: string;
  task: CodeGitChangeSet;
  staged: CodeGitChangeSet;
  unstaged: CodeGitChangeSet;
  hasConflicts: boolean;
  commitIdentity: { name: string; email: string } | null;
  capabilities: {
    stage: CodeGitCapability;
    unstage: CodeGitCapability;
    commit: CodeGitCapability;
  };
  blockingReceipt: CodeGitMutationReceipt | null;
};

export type CodeGitStatus =
  | CodeGitReadyStatus
  | {
      state: "blocked";
      runtimeGeneration: number;
      statusRevision: number;
      writeGeneration: number;
      scope: CodeThreadBindingScope;
      threadId: string;
      reason: string;
      remediation: string;
    }
  | {
      state: "recoveryRequired";
      runtimeGeneration: number;
      statusRevision: number;
      writeGeneration: number;
      scope: CodeThreadBindingScope;
      threadId: string;
      operation: {
        operationId: string;
        operation: CodeGitOperation;
        journalState: "pending" | "recovering" | "uncertain";
      };
    };

export type CodeGitReconcileResult =
  | ({ state: "none" } & CodeGitStatusInput)
  | ({
      state: "pending" | "recovering";
      operationId: string;
      operation: CodeGitOperation;
    } & CodeGitStatusInput)
  | { state: "completed"; receipt: CodeGitMutationReceipt }
  | ({
      state: "uncertain";
      operationId: string;
      operation: CodeGitOperation;
      message: string;
    } & CodeGitStatusInput);

export type CodeGitAcknowledgeReceipt = CodeGitStatusInput & {
  operationId: string;
  disposition: "acknowledged";
};
