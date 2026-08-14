import { queryOptions } from "@tanstack/react-query";

import { codeWorkspaceApi, type CodeWorkspaceApi } from "../api/codeWorkspace";
import type {
  CodeRepositoryInspectInput,
  CodeThreadBindingScope,
  CodeWorktreeDescriptor,
} from "../api/types";

/** Scalar query keys keep every native scope coordinate explicit. */
export const codeSessionQueryKeys = {
  all: ["schoolx-code"] as const,
  runtime: () => ["schoolx-code", "runtime"] as const,
  runtimeProbe: () => ["schoolx-code", "runtime", "probe"] as const,
  runtimeStatus: () => ["schoolx-code", "runtime", "status"] as const,
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
  threads: (scope: CodeThreadBindingScope) =>
    [...codeSessionQueryKeys.scope(scope), "threads"] as const,
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
