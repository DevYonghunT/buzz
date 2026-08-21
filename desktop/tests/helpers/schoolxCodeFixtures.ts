import type {
  CodeGitMutationReceipt,
  CodeGitReadyStatus,
} from "../../src/features/code/api/codeGitTypes";
import type { CodeThreadChanges } from "../../src/features/code/api/types";

export const SCHOOLX_CREATED_THREAD_ID = "thread-e2e-new";
export const SCHOOLX_CODE_SCOPE = {
  communityId: "e2e-default-community",
  projectDtag: "buzz",
  repositoryIdentity: "c4".repeat(32),
};

export function gitReadyStatus({
  blockingReceipt = null,
  clean = false,
  staged,
  statusRevision,
  writeGeneration,
}: {
  blockingReceipt?: CodeGitMutationReceipt | null;
  clean?: boolean;
  staged: boolean;
  statusRevision: number;
  writeGeneration: number;
}): CodeGitReadyStatus {
  const file = {
    fileId: (staged ? "b" : "a").repeat(64),
    path: "desktop/src/features/code/ui/CodeChangesPanel.tsx",
    status: "modified" as const,
    binary: false,
    additions: 4,
    deletions: 1,
    patch: "@@ -1 +1 @@\n-old\n+new",
    truncated: false,
  };
  const empty = {
    files: [],
    totalFiles: 0,
    filesTruncated: false,
    additions: 0,
    deletions: 0,
  };
  const populated = {
    files: [file],
    totalFiles: 1,
    filesTruncated: false,
    additions: 4,
    deletions: 1,
  };
  return {
    state: "ready",
    runtimeGeneration: 7,
    statusRevision,
    writeGeneration,
    snapshotSequence: statusRevision,
    scope: SCHOOLX_CODE_SCOPE,
    threadId: SCHOOLX_CREATED_THREAD_ID,
    snapshotId: (staged ? "d" : clean ? "9" : "c").repeat(64),
    headCommit: (clean ? "2" : "1").repeat(40),
    task: populated,
    staged: staged ? populated : empty,
    unstaged: staged || clean ? empty : populated,
    hasConflicts: false,
    commitIdentity: { name: "SchoolX User", email: "user@example.com" },
    capabilities: {
      stage: {
        enabled: blockingReceipt === null && !staged && !clean,
        reason:
          blockingReceipt !== null
            ? "A completed operation is awaiting acknowledgement."
            : staged || clean
              ? "There are no unstaged files."
              : null,
      },
      unstage: {
        enabled: blockingReceipt === null && staged,
        reason:
          blockingReceipt !== null
            ? "A completed operation is awaiting acknowledgement."
            : staged
              ? null
              : "There are no staged files.",
      },
      commit: {
        enabled: blockingReceipt === null && staged,
        reason:
          blockingReceipt !== null
            ? "A completed operation is awaiting acknowledgement."
            : staged
              ? null
              : "There are no staged files.",
      },
    },
    blockingReceipt,
  };
}

export const PARTIAL_THREAD_CHANGES: CodeThreadChanges = {
  files: [
    {
      path: "desktop/src/features/code/ui/CodeChangesPanel.tsx",
      status: "modified",
      binary: false,
      additions: 4,
      deletions: 1,
      patch: [
        "@@ -1,2 +1,3 @@",
        "-const complete = false;",
        "+const complete = true;",
      ].join("\n"),
      truncated: true,
    },
    {
      path: "docs/schoolx-2/obsolete.md",
      status: "deleted",
      binary: false,
      additions: 0,
      deletions: 2,
      patch: "@@ -1,2 +0,0 @@\n-obsolete line\n-old detail",
      truncated: false,
    },
    {
      path: "desktop/assets/code-workspace.bin",
      status: "untracked",
      binary: true,
      additions: 0,
      deletions: 0,
      patch: "",
      truncated: false,
    },
  ],
  additions: 4,
  deletions: 3,
  commitBody: null,
  totalFiles: 5,
  filesTruncated: true,
};

export const STALE_THREAD_CHANGES: CodeThreadChanges = {
  files: [
    {
      path: "desktop/src/features/code/staleSnapshot.ts",
      status: "modified",
      binary: false,
      additions: 1,
      deletions: 0,
      patch: "@@ -0,0 +1 @@\n+export const staleSnapshot = true;",
      truncated: false,
    },
  ],
  additions: 1,
  deletions: 0,
  commitBody: null,
  totalFiles: 1,
  filesTruncated: false,
};

export const FRESH_THREAD_CHANGES: CodeThreadChanges = {
  files: [
    {
      path: "desktop/src/features/code/freshSnapshot.ts",
      status: "added",
      binary: false,
      additions: 1,
      deletions: 0,
      patch: "@@ -0,0 +1 @@\n+export const freshSnapshot = true;",
      truncated: false,
    },
  ],
  additions: 1,
  deletions: 0,
  commitBody: null,
  totalFiles: 1,
  filesTruncated: false,
};
