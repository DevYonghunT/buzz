import type { QueryClient } from "@tanstack/react-query";
import * as React from "react";

import type { CodeThreadBindingScope } from "../api/types";
import { codeSessionQueryKeys } from "./codeSessionQueries";

export function useCodeWorkspaceListRefresh(
  queryClient: QueryClient,
  scope: CodeThreadBindingScope,
  mountedRef: React.RefObject<boolean>,
) {
  const [pending, setPending] = React.useState(false);
  const inFlightRef = React.useRef<Promise<void> | null>(null);

  const refresh = React.useCallback((): Promise<void> => {
    const activeRefresh = inFlightRef.current;
    if (activeRefresh) return activeRefresh;

    setPending(true);
    const nextRefresh = Promise.all([
      queryClient.invalidateQueries({
        queryKey: codeSessionQueryKeys.preparations(scope),
      }),
      queryClient.invalidateQueries({
        queryKey: codeSessionQueryKeys.threads(scope),
      }),
      queryClient.invalidateQueries({
        queryKey: codeSessionQueryKeys.worktrees(scope),
      }),
    ]).then(() => undefined);
    inFlightRef.current = nextRefresh;

    const settle = () => {
      if (inFlightRef.current !== nextRefresh) return;
      inFlightRef.current = null;
      if (mountedRef.current) setPending(false);
    };
    void nextRefresh.then(settle, settle);
    return nextRefresh;
  }, [mountedRef, queryClient, scope]);

  return { pending, refresh };
}
