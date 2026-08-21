import { queryOptions } from "@tanstack/react-query";

import { codeWorkspaceApi, type CodeWorkspaceApi } from "../api/codeWorkspace";
import type {
  CodeRepositoryInspectInput,
  CodeThreadBindingScope,
  CodeThreadChangesInput,
  CodeWorktreeDescriptor,
} from "../api/types";
import type { CodeGitStatusInput } from "../api/codeGitTypes";
import type { CodeGitHandoffAttempt } from "./codeGitHandoffMachine";

/** Cache identity for one thread diff within an exact runtime generation. */
export type CodeThreadChangesQueryIdentity = CodeThreadChangesInput & {
  readonly runtimeGeneration: number;
};

/** Scalar query keys keep every native scope coordinate explicit. */
export const codeSessionQueryKeys = {
  all: ["schoolx-code"] as const,
  runtime: () => ["schoolx-code", "runtime"] as const,
  runtimeProbe: () => ["schoolx-code", "runtime", "probe"] as const,
  runtimeStatus: () => ["schoolx-code", "runtime", "status"] as const,
  models: (runtimeGeneration: number) =>
    ["schoolx-code", "runtime", "models", runtimeGeneration] as const,
  repository: (input: CodeRepositoryInspectInput) =>
    [
      "schoolx-code",
      "repository",
      input.repositoryRoot,
      input.baseRef,
    ] as const,
  scope: (scope: CodeThreadBindingScope) =>
    [
      "schoolx-code",
      "scope",
      scope.communityId,
      scope.projectDtag,
      scope.repositoryIdentity,
    ] as const,
  preparations: (scope: CodeThreadBindingScope) =>
    [...codeSessionQueryKeys.scope(scope), "preparations"] as const,
  worktrees: (scope: CodeThreadBindingScope) =>
    [...codeSessionQueryKeys.scope(scope), "worktrees"] as const,
  worktreeRemovalAttempt: (scope: CodeThreadBindingScope) =>
    [...codeSessionQueryKeys.scope(scope), "worktree-removal-attempt"] as const,
  threads: (scope: CodeThreadBindingScope) =>
    [...codeSessionQueryKeys.scope(scope), "threads"] as const,
  threadChanges: (identity: CodeThreadChangesQueryIdentity) =>
    [
      ...codeSessionQueryKeys.scope(identity.scope),
      "thread-changes",
      identity.threadId,
      identity.runtimeGeneration,
    ] as const,
  threadGitStatus: (
    identity: CodeGitStatusInput & { runtimeGeneration: number },
  ) =>
    [
      ...codeSessionQueryKeys.scope(identity.scope),
      "thread-git-status",
      identity.threadId,
      identity.runtimeGeneration,
    ] as const,
  threadGitAttempt: (identity: CodeGitStatusInput) =>
    [
      ...codeSessionQueryKeys.scope(identity.scope),
      "thread-git-attempt",
      identity.threadId,
    ] as const,
  worktreeStatus: (descriptor: CodeWorktreeDescriptor) =>
    [
      "schoolx-code",
      "worktree-status",
      descriptor.executionMode,
      descriptor.repositoryIdentity,
      descriptor.executionRoot,
      descriptor.baseRef,
      descriptor.worktreeId,
    ] as const,
};

/** Query options for Codex executable discovery. */
export function codeRuntimeProbeQueryOptions(
  api: CodeWorkspaceApi = codeWorkspaceApi,
) {
  return queryOptions({
    queryKey: codeSessionQueryKeys.runtimeProbe(),
    queryFn: () => api.probeCodeRuntime(),
  });
}

/** Query options for the desktop-wide app-server lifecycle snapshot. */
export function codeRuntimeStatusQueryOptions(
  api: CodeWorkspaceApi = codeWorkspaceApi,
) {
  return queryOptions({
    queryKey: codeSessionQueryKeys.runtimeStatus(),
    queryFn: () => api.getCodeRuntimeStatus(),
  });
}

/** Query options for the normalized model catalog of one runtime generation. */
export function codeModelsQueryOptions(
  runtimeGeneration: number,
  api: CodeWorkspaceApi = codeWorkspaceApi,
) {
  return queryOptions({
    queryKey: codeSessionQueryKeys.models(runtimeGeneration),
    queryFn: async () => {
      const catalog = await api.listCodeModels();
      if (catalog.runtimeGeneration !== runtimeGeneration) {
        throw new TypeError(
          "Code model catalog must match the requested runtime generation",
        );
      }
      return catalog;
    },
    retry: false,
    staleTime: Number.POSITIVE_INFINITY,
  });
}

/** Query options for read-only native Git identity and base-ref validation. */
export function codeRepositoryQueryOptions(
  input: CodeRepositoryInspectInput,
  api: CodeWorkspaceApi = codeWorkspaceApi,
) {
  return queryOptions({
    queryKey: codeSessionQueryKeys.repository(input),
    queryFn: () => api.inspectCodeRepository(input),
  });
}

/** Query options for unfinished preparations in one exact native scope. */
export function codeThreadPreparationsQueryOptions(
  scope: CodeThreadBindingScope,
  api: CodeWorkspaceApi = codeWorkspaceApi,
) {
  return queryOptions({
    queryKey: codeSessionQueryKeys.preparations(scope),
    queryFn: () => api.listCodeThreadPreparations({ scope }),
  });
}

/** Query options for the read-only managed-worktree preservation inventory. */
export function codeWorktreesQueryOptions(
  scope: CodeThreadBindingScope,
  api: CodeWorkspaceApi = codeWorkspaceApi,
) {
  return queryOptions({
    queryKey: codeSessionQueryKeys.worktrees(scope),
    queryFn: () => api.listCodeWorktrees({ scope }),
    retryOnMount: false,
  });
}

/** Query options for durable thread bindings in one exact native scope. */
export function codeThreadsQueryOptions(
  scope: CodeThreadBindingScope,
  api: CodeWorkspaceApi = codeWorkspaceApi,
) {
  return queryOptions({
    queryKey: codeSessionQueryKeys.threads(scope),
    queryFn: () => api.listCodeThreads({ scope }),
  });
}

/** Query options for the current diff of one exact persisted thread binding. */
export function codeThreadChangesQueryOptions(
  identity: CodeThreadChangesQueryIdentity,
  api: CodeWorkspaceApi = codeWorkspaceApi,
) {
  return queryOptions({
    queryKey: codeSessionQueryKeys.threadChanges(identity),
    queryFn: () =>
      api.getCodeThreadChanges({
        scope: identity.scope,
        threadId: identity.threadId,
      }),
  });
}

/** Authoritative staged/unstaged snapshot for one exact runtime generation. */
export function codeThreadGitStatusQueryOptions(
  identity: CodeGitStatusInput & { runtimeGeneration: number },
  api: CodeWorkspaceApi = codeWorkspaceApi,
) {
  return queryOptions({
    queryKey: codeSessionQueryKeys.threadGitStatus(identity),
    queryFn: async () => {
      const status = await api.getCodeThreadGitStatus({
        scope: identity.scope,
        threadId: identity.threadId,
      });
      if (
        status.runtimeGeneration !== identity.runtimeGeneration ||
        status.threadId !== identity.threadId
      ) {
        throw new TypeError(
          "Code Git status must match the requested runtime and thread",
        );
      }
      return status;
    },
    retry: false,
  });
}

/** Community-scoped local handoff state, retained across component remounts. */
export function codeThreadGitAttemptQueryOptions(identity: CodeGitStatusInput) {
  return queryOptions<CodeGitHandoffAttempt | null>({
    queryKey: codeSessionQueryKeys.threadGitAttempt(identity),
    queryFn: async () => null,
    enabled: false,
    gcTime: Number.POSITIVE_INFINITY,
    initialData: null,
    staleTime: Number.POSITIVE_INFINITY,
  });
}

/** Query options for read-only revalidation of one native execution root. */
export function codeWorktreeStatusQueryOptions(
  descriptor: CodeWorktreeDescriptor,
  api: CodeWorkspaceApi = codeWorkspaceApi,
) {
  return queryOptions({
    queryKey: codeSessionQueryKeys.worktreeStatus(descriptor),
    queryFn: () => api.getCodeWorktreeStatus(descriptor),
  });
}
