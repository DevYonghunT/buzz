import * as React from "react";

import {
  codePanelWidthFromKey,
  codePanelWidthFromPointer,
} from "../lib/codePanelLayout";
import { cn } from "@/shared/lib/cn";

type DragState = {
  pointerId: number;
  previousCursor: string;
  previousUserSelect: string;
  startWidth: number;
  startX: number;
};

export function CodePanelResizeHandle({
  ariaLabel,
  className,
  growDirection,
  max,
  min,
  onChange,
  testId,
  value,
}: {
  ariaLabel: string;
  className?: string;
  growDirection: 1 | -1;
  max: number;
  min: number;
  onChange: (width: number) => void;
  testId: string;
  value: number;
}) {
  const dragRef = React.useRef<DragState | null>(null);
  const [dragging, setDragging] = React.useState(false);
  const separatorPosition = growDirection === 1 ? value : min + max - value;

  React.useEffect(
    () => () => {
      const drag = dragRef.current;
      if (!drag) return;
      document.documentElement.style.cursor = drag.previousCursor;
      document.body.style.userSelect = drag.previousUserSelect;
      dragRef.current = null;
    },
    [],
  );

  const finishDrag = React.useCallback(
    (element: HTMLHRElement, pointerId: number, release: boolean) => {
      const drag = dragRef.current;
      if (!drag || drag.pointerId !== pointerId) return;
      dragRef.current = null;
      document.documentElement.style.cursor = drag.previousCursor;
      document.body.style.userSelect = drag.previousUserSelect;
      setDragging(false);
      if (release && element.hasPointerCapture(pointerId)) {
        element.releasePointerCapture(pointerId);
      }
    },
    [],
  );

  return (
    <hr
      aria-label={ariaLabel}
      aria-orientation="vertical"
      aria-valuemax={Math.round(max)}
      aria-valuemin={Math.round(min)}
      aria-valuenow={Math.round(separatorPosition)}
      aria-valuetext={`${Math.round(value)} pixels`}
      className={cn(
        "relative z-30 m-0 h-full w-3 shrink-0 touch-none cursor-col-resize border-0 bg-transparent outline-hidden before:absolute before:inset-y-0 before:left-1/2 before:w-px before:-translate-x-1/2 before:bg-border/60 hover:before:bg-primary/60 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset focus-visible:before:bg-primary/70 data-[resizing=true]:before:bg-primary",
        className,
      )}
      data-resizing={dragging ? "true" : "false"}
      data-testid={testId}
      onKeyDown={(event) => {
        const next = codePanelWidthFromKey({
          growDirection,
          key: event.key,
          max,
          min,
          width: value,
        });
        if (next === null) return;
        event.preventDefault();
        onChange(next);
      }}
      onLostPointerCapture={(event) =>
        finishDrag(event.currentTarget, event.pointerId, false)
      }
      onPointerCancel={(event) =>
        finishDrag(event.currentTarget, event.pointerId, true)
      }
      onPointerDown={(event) => {
        if (event.button !== 0 || !event.isPrimary) return;
        event.preventDefault();
        event.currentTarget.focus();
        dragRef.current = {
          pointerId: event.pointerId,
          previousCursor: document.documentElement.style.cursor,
          previousUserSelect: document.body.style.userSelect,
          startWidth: value,
          startX: event.clientX,
        };
        event.currentTarget.setPointerCapture(event.pointerId);
        document.documentElement.style.cursor = "col-resize";
        document.body.style.userSelect = "none";
        setDragging(true);
      }}
      onPointerMove={(event) => {
        const drag = dragRef.current;
        if (!drag || drag.pointerId !== event.pointerId) return;
        event.preventDefault();
        onChange(
          codePanelWidthFromPointer({
            currentX: event.clientX,
            growDirection,
            max,
            min,
            startWidth: drag.startWidth,
            startX: drag.startX,
          }),
        );
      }}
      onPointerUp={(event) =>
        finishDrag(event.currentTarget, event.pointerId, true)
      }
      tabIndex={0}
    />
  );
}
