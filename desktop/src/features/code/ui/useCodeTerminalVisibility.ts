import * as React from "react";

import type { CodeThreadBindingScope } from "../api/types";
import { useCodeTerminalShortcut } from "./CodeTerminalDrawer";

export function useCodeTerminalVisibility(
  scope: CodeThreadBindingScope,
  threadId: string | null,
) {
  const ownerKey = threadId
    ? JSON.stringify([
        scope.communityId,
        scope.projectDtag,
        scope.repositoryIdentity,
        threadId,
      ])
    : null;
  const [visibility, setVisibility] = React.useState<{
    ownerKey: string | null;
    open: boolean;
  }>({ ownerKey: null, open: false });
  const open = visibility.ownerKey === ownerKey && visibility.open;

  React.useLayoutEffect(() => {
    setVisibility((current) =>
      current.ownerKey === ownerKey ? current : { ownerKey, open: false },
    );
  }, [ownerKey]);

  const setOpen = React.useCallback(
    (nextOpen: boolean) => {
      setVisibility({ ownerKey, open: ownerKey === null ? false : nextOpen });
    },
    [ownerKey],
  );
  const toggle = React.useCallback(() => {
    if (ownerKey === null) return;
    setVisibility((current) => ({
      ownerKey,
      open: current.ownerKey === ownerKey ? !current.open : true,
    }));
  }, [ownerKey]);

  useCodeTerminalShortcut(toggle, ownerKey !== null);

  return { open, ownerKey, setOpen, toggle };
}
