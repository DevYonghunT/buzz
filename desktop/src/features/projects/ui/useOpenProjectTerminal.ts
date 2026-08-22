import { useQueryClient } from "@tanstack/react-query";
import * as React from "react";
import { toast } from "sonner";

import type { Project } from "@/features/projects/hooks";
import { openProjectTerminal } from "@/shared/api/projectGit";

export function projectTerminalLabel(hasLocalCheckout: boolean) {
  return hasLocalCheckout ? "Open in Terminal" : "Clone & open in Terminal";
}

/**
 * Opens the OS terminal at a project's local checkout, cloning first when
 * only a remote exists. Handles the clone progress/success/error toasts and
 * refreshes project queries after a missing checkout is resolved so the UI
 * also recovers when another actor completed the clone first.
 */
export function useOpenProjectTerminal(reposDir?: string | null) {
  const queryClient = useQueryClient();

  return React.useCallback(
    async (
      project: Project,
      options: { branch?: string | null; hasLocalCheckout: boolean },
    ) => {
      const toastId = options.hasLocalCheckout
        ? undefined
        : toast.loading(`Cloning ${project.name}…`);
      try {
        const result = await openProjectTerminal({
          reposDir,
          projectDtag: project.dtag,
          cloneUrl: project.cloneUrls[0] ?? null,
          defaultBranch: options.branch ?? project.defaultBranch ?? null,
        });
        if (result.cloned) {
          toast.success(`Cloned to ${result.path}`, { id: toastId });
        } else if (toastId !== undefined) {
          toast.dismiss(toastId);
        }

        if (result.cloned || !options.hasLocalCheckout) {
          void queryClient.invalidateQueries({
            queryKey: ["project", project.id],
          });
          void queryClient.invalidateQueries({ queryKey: ["projects"] });
        }
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to open terminal",
          { id: toastId },
        );
      }
    },
    [queryClient, reposDir],
  );
}
