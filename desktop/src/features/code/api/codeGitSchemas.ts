import { z } from "zod";

import { CodeThreadBindingScopeSchema } from "./schemas";

const opaqueId = z.string().regex(/^[0-9a-f]{64}$/);
const oid = z.string().regex(/^[0-9a-f]{40,64}$/);
const count = z.number().int().nonnegative();

export const CodeGitStatusInputSchema = z
  .object({ scope: CodeThreadBindingScopeSchema, threadId: z.string().min(1) })
  .strict();
export const CodeGitIndexMutationInputSchema = CodeGitStatusInputSchema.extend({
  writeGeneration: count,
  snapshotId: opaqueId,
  fileId: opaqueId,
}).strict();
export const CodeGitCommitInputSchema = CodeGitStatusInputSchema.extend({
  writeGeneration: count,
  snapshotId: opaqueId,
  message: z.string().min(1).max(65_536),
}).strict();
export const CodeGitAcknowledgeInputSchema = CodeGitStatusInputSchema.extend({
  operationId: opaqueId,
  writeGeneration: count,
  snapshotId: opaqueId,
}).strict();

const CodeGitChangeFileSchema = z
  .object({
    fileId: opaqueId,
    path: z.string().min(1),
    status: z.enum([
      "added",
      "modified",
      "deleted",
      "typeChanged",
      "unmerged",
      "untracked",
    ]),
    binary: z.boolean(),
    additions: count,
    deletions: count,
    patch: z.string(),
    truncated: z.boolean(),
  })
  .strict();

export const CodeGitChangeSetSchema = z
  .object({
    files: z.array(CodeGitChangeFileSchema).max(250),
    totalFiles: count,
    filesTruncated: z.boolean(),
    additions: count,
    deletions: count,
  })
  .strict()
  .superRefine((value, context) => {
    if (value.totalFiles !== value.files.length) {
      context.addIssue({
        code: "custom",
        message: "Git action manifest must be complete",
      });
    }
    if (value.filesTruncated) {
      context.addIssue({
        code: "custom",
        message: "Git action manifest cannot be truncated",
      });
    }
    if (
      value.additions !==
      value.files.reduce((sum, file) => sum + file.additions, 0)
    ) {
      context.addIssue({
        code: "custom",
        message: "Git additions total does not match files",
      });
    }
    if (
      value.deletions !==
      value.files.reduce((sum, file) => sum + file.deletions, 0)
    ) {
      context.addIssue({
        code: "custom",
        message: "Git deletions total does not match files",
      });
    }
    const paths = value.files.map((file) => file.path);
    if (
      new Set(paths).size !== paths.length ||
      paths.some((path, index) => index > 0 && paths[index - 1] >= path)
    ) {
      context.addIssue({
        code: "custom",
        message: "Git paths must be unique and sorted",
      });
    }
  });

const CodeGitTaskChangeSetSchema = z
  .object({
    files: z.array(CodeGitChangeFileSchema).max(250),
    totalFiles: count,
    filesTruncated: z.boolean(),
    additions: count,
    deletions: count,
  })
  .strict()
  .superRefine((value, context) => {
    if (
      (!value.filesTruncated && value.totalFiles !== value.files.length) ||
      (value.filesTruncated && value.totalFiles <= value.files.length)
    ) {
      context.addIssue({
        code: "custom",
        message: "Task diff completeness metadata is inconsistent",
      });
    }
    const paths = value.files.map((file) => file.path);
    if (
      new Set(paths).size !== paths.length ||
      paths.some((path, index) => index > 0 && paths[index - 1] >= path)
    ) {
      context.addIssue({
        code: "custom",
        message: "Task diff paths must be unique and sorted",
      });
    }
  });

const capability = z
  .object({ enabled: z.boolean(), reason: z.string().min(1).nullable() })
  .strict()
  .superRefine((value, context) => {
    if (value.enabled !== (value.reason === null)) {
      context.addIssue({
        code: "custom",
        message: "Enabled Git capability must have a null reason",
      });
    }
  });

export const CodeGitIndexMutationReceiptSchema = z
  .object({
    operationId: opaqueId,
    operation: z.enum(["stage", "unstage"]),
    scope: CodeThreadBindingScopeSchema,
    threadId: z.string().min(1),
    requestGeneration: count,
    beforeSnapshotId: opaqueId,
    fileId: opaqueId,
    disposition: z.enum(["staged", "unstaged"]),
  })
  .strict();
export const CodeGitCommitReceiptSchema = z
  .object({
    operationId: opaqueId,
    operation: z.literal("commit"),
    scope: CodeThreadBindingScopeSchema,
    threadId: z.string().min(1),
    requestGeneration: count,
    beforeSnapshotId: opaqueId,
    previousHead: oid,
    commit: oid,
    tree: oid,
    disposition: z.literal("committed"),
  })
  .strict();
export const CodeGitMutationReceiptSchema = z.union([
  CodeGitIndexMutationReceiptSchema,
  CodeGitCommitReceiptSchema,
]);

const ready = z
  .object({
    state: z.literal("ready"),
    runtimeGeneration: count,
    statusRevision: count,
    writeGeneration: count,
    snapshotSequence: count,
    scope: CodeThreadBindingScopeSchema,
    threadId: z.string().min(1),
    snapshotId: opaqueId,
    headCommit: oid,
    task: CodeGitTaskChangeSetSchema,
    staged: CodeGitChangeSetSchema,
    unstaged: CodeGitChangeSetSchema,
    hasConflicts: z.boolean(),
    commitIdentity: z
      .object({ name: z.string().min(1), email: z.string().min(3) })
      .strict()
      .nullable(),
    capabilities: z
      .object({ stage: capability, unstage: capability, commit: capability })
      .strict(),
    blockingReceipt: CodeGitMutationReceiptSchema.nullable(),
  })
  .strict()
  .superRefine((value, context) => {
    const byPath = new Map<string, string>();
    for (const file of [...value.staged.files, ...value.unstaged.files]) {
      const prior = byPath.get(file.path);
      if (prior !== undefined && prior !== file.fileId) {
        context.addIssue({
          code: "custom",
          message: "Partially staged path must share one file ID",
        });
      }
      byPath.set(file.path, file.fileId);
    }
  });
const blocked = z
  .object({
    state: z.literal("blocked"),
    runtimeGeneration: count,
    statusRevision: count,
    writeGeneration: count,
    scope: CodeThreadBindingScopeSchema,
    threadId: z.string().min(1),
    reason: z.string().min(1),
    remediation: z.string().min(1),
  })
  .strict();
const recoveryRequired = z
  .object({
    state: z.literal("recoveryRequired"),
    runtimeGeneration: count,
    statusRevision: count,
    writeGeneration: count,
    scope: CodeThreadBindingScopeSchema,
    threadId: z.string().min(1),
    operation: z
      .object({
        operationId: opaqueId,
        operation: z.enum(["stage", "unstage", "commit"]),
        journalState: z.enum(["pending", "recovering", "uncertain"]),
      })
      .strict(),
  })
  .strict();
export const CodeGitStatusSchema = z.discriminatedUnion("state", [
  ready,
  blocked,
  recoveryRequired,
]);

export const CodeGitReconcileResultSchema = z.discriminatedUnion("state", [
  CodeGitStatusInputSchema.extend({ state: z.literal("none") }).strict(),
  CodeGitStatusInputSchema.extend({
    state: z.literal("pending"),
    operationId: opaqueId,
    operation: z.enum(["stage", "unstage", "commit"]),
  }).strict(),
  CodeGitStatusInputSchema.extend({
    state: z.literal("recovering"),
    operationId: opaqueId,
    operation: z.enum(["stage", "unstage", "commit"]),
  }).strict(),
  z
    .object({
      state: z.literal("completed"),
      receipt: CodeGitMutationReceiptSchema,
    })
    .strict(),
  CodeGitStatusInputSchema.extend({
    state: z.literal("uncertain"),
    operationId: opaqueId,
    operation: z.enum(["stage", "unstage", "commit"]),
    message: z.string().min(1),
  }).strict(),
]);

export const CodeGitAcknowledgeReceiptSchema = CodeGitStatusInputSchema.extend({
  operationId: opaqueId,
  disposition: z.literal("acknowledged"),
}).strict();
