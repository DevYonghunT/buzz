import type { QueryClient } from "@tanstack/react-query";

import {
  codeWorkspaceApi,
  type CodeWorkspaceApi,
} from "@/features/code/api/codeWorkspace";
import { codeSessionQueryKeys } from "@/features/code/state/codeSessionQueries";

/** Only a completed Codex install should trigger SchoolX Code discovery. */
export function shouldRefreshCodeRuntimeAfterInstall(
  runtimeId: string,
  result: { success: boolean } | undefined,
  error: unknown,
): boolean {
  return runtimeId === "codex" && error === null && result?.success === true;
}

/** Re-discover Codex immediately after its managed installer finishes. */
export async function refreshCodeRuntimeAfterInstall(
  runtimeId: string,
  queryClient: Pick<QueryClient, "invalidateQueries" | "setQueryData">,
  api: Pick<CodeWorkspaceApi, "probeCodeRuntime"> = codeWorkspaceApi,
): Promise<void> {
  if (runtimeId !== "codex") return;

  const probe = await api.probeCodeRuntime();
  queryClient.setQueryData(codeSessionQueryKeys.runtimeProbe(), probe);
  await queryClient.invalidateQueries({
    queryKey: codeSessionQueryKeys.runtimeStatus(),
  });
}
