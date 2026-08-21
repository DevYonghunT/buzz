import type { QueryClient } from "@tanstack/react-query";
import * as React from "react";

import type { CodeThreadBindingScope, CodeWorkspaceEvent } from "../api/types";
import { codeSessionQueryKeys } from "./codeSessionQueries";

export function useCodeThreadLifecycleSync({
  acknowledgeAuthoritativeForkRefresh,
  acknowledgeAuthoritativeListRefresh,
  events,
  preparationsDataUpdatedAt,
  preparationsQuerySucceeded,
  queryClient,
  scope,
  threadsDataUpdatedAt,
  threadsQuerySucceeded,
}: {
  acknowledgeAuthoritativeForkRefresh: () => void;
  acknowledgeAuthoritativeListRefresh: () => void;
  events: readonly CodeWorkspaceEvent[];
  preparationsDataUpdatedAt: number;
  preparationsQuerySucceeded: boolean;
  queryClient: QueryClient;
  scope: CodeThreadBindingScope;
  threadsDataUpdatedAt: number;
  threadsQuerySucceeded: boolean;
}) {
  const acknowledgedThreadsUpdateRef = React.useRef(0);
  React.useEffect(() => {
    if (
      threadsQuerySucceeded &&
      threadsDataUpdatedAt > acknowledgedThreadsUpdateRef.current
    ) {
      acknowledgedThreadsUpdateRef.current = threadsDataUpdatedAt;
      acknowledgeAuthoritativeListRefresh();
    }
  }, [
    acknowledgeAuthoritativeListRefresh,
    threadsDataUpdatedAt,
    threadsQuerySucceeded,
  ]);

  const acknowledgedForkUpdateRef = React.useRef("");
  React.useEffect(() => {
    if (!threadsQuerySucceeded || !preparationsQuerySucceeded) return;
    const identity = `${threadsDataUpdatedAt}:${preparationsDataUpdatedAt}`;
    if (acknowledgedForkUpdateRef.current === identity) return;
    acknowledgedForkUpdateRef.current = identity;
    acknowledgeAuthoritativeForkRefresh();
  }, [
    acknowledgeAuthoritativeForkRefresh,
    preparationsDataUpdatedAt,
    preparationsQuerySucceeded,
    threadsDataUpdatedAt,
    threadsQuerySucceeded,
  ]);

  const notificationIdentity = React.useMemo(() => {
    let notification: CodeWorkspaceEvent | undefined;
    for (let index = events.length - 1; index >= 0; index -= 1) {
      const event = events[index];
      if (
        event?.kind === "thread/archived" ||
        event?.kind === "thread/unarchived"
      ) {
        notification = event;
        break;
      }
    }
    return notification
      ? `${notification.runtimeGeneration}:${notification.sequence}`
      : null;
  }, [events]);
  const notificationRef = React.useRef<string | null>(null);
  React.useEffect(() => {
    if (
      notificationIdentity === null ||
      notificationRef.current === notificationIdentity
    ) {
      return;
    }
    notificationRef.current = notificationIdentity;
    void queryClient.invalidateQueries({
      exact: true,
      queryKey: codeSessionQueryKeys.threads(scope),
    });
  }, [notificationIdentity, queryClient, scope]);
}
