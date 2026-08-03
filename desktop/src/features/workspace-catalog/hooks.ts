import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  applyWorkspaceCatalog,
  preflightWorkspaceCatalog,
} from "@/shared/api/tauriWorkspaceCatalog";
import { isCatalogGateRefusalError } from "./catalogError";

export const workspaceCatalogPreflightQueryKey = [
  "workspace-catalog",
  "preflight",
] as const;

export function useWorkspaceCatalogPreflightQuery() {
  return useQuery({
    queryKey: workspaceCatalogPreflightQueryKey,
    queryFn: preflightWorkspaceCatalog,
    // Opt out of the global `retry: 1` (`shared/api/queryClient.ts`) for the
    // permission refusals only. They are deterministic verdicts about who the
    // caller is, so the retry cannot succeed — it just holds the skeleton for
    // another round-trip before the card can explain. Everything else (relay
    // unreachable, timeouts) keeps the one retry, which is what it is for.
    retry: (failureCount, error) =>
      !isCatalogGateRefusalError(error) && failureCount < 1,
  });
}

/**
 * Applying changes what the next preflight would report (e.g. an item moves
 * from `create_or_recreate` to `no_change`), so a successful — or partial —
 * run invalidates the preflight query on settle. The ledger returned here is
 * the point-in-time result of *this* apply; the card keeps it in local state
 * to render per-item outcomes, separate from the live (refetched) decision.
 */
export function useApplyWorkspaceCatalogMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (selected: string[]) => applyWorkspaceCatalog(selected),
    onSettled: async () => {
      await queryClient.invalidateQueries({
        queryKey: workspaceCatalogPreflightQueryKey,
      });
    },
  });
}
