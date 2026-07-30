import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  applyWorkspaceCatalog,
  preflightWorkspaceCatalog,
} from "@/shared/api/tauriWorkspaceCatalog";

export const workspaceCatalogPreflightQueryKey = [
  "workspace-catalog",
  "preflight",
] as const;

export function useWorkspaceCatalogPreflightQuery() {
  return useQuery({
    queryKey: workspaceCatalogPreflightQueryKey,
    queryFn: preflightWorkspaceCatalog,
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
