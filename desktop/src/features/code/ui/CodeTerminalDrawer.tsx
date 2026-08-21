import { FitAddon } from "@xterm/addon-fit";
import { type ITheme, Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { ChevronDown, RotateCcw, Square, SquareTerminal } from "lucide-react";
import * as React from "react";

import {
  openCodeTerminal,
  resizeCodeTerminal,
  terminateCodeTerminal,
  writeCodeTerminalStdin,
} from "../api/codeWorkspace";
import {
  codeScopesEqual,
  type CodeTerminalEvent,
  type CodeTerminalSession,
  type CodeThreadBindingScope,
} from "../api/types";
import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";
import { useTheme } from "@/shared/theme/ThemeProvider";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";

const MAX_STDIN_CHUNK_BYTES = 64 * 1024;
const MAX_TERMINAL_DIMENSION = 1_000;
// Match the app's stock `text-sm` step while deriving xterm's required px value.
const TERMINAL_FONT_REM = 0.875;

type TerminalPhase =
  | "idle"
  | "starting"
  | "running"
  | "terminating"
  | "exited"
  | "error";

type ExitStatus = {
  exitCode: number;
  signal: string | null;
};

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function terminalTheme(isDark = false, accentFallback = "#c678dd"): ITheme {
  const styles = window.getComputedStyle(document.documentElement);
  const color = (name: string, fallback: string, opacity?: number) => {
    const value = styles.getPropertyValue(name).trim();
    if (!value) return fallback;
    return `hsl(${value}${opacity === undefined ? "" : ` / ${opacity}`})`;
  };

  const background = color("--background", isDark ? "#111111" : "#f3f3f3");
  const foreground = color("--foreground", isDark ? "#eeeeee" : "#333333");
  const muted = color("--muted-foreground", "#888888");
  const black = isDark ? color("--muted", "#555555") : foreground;
  const red = color("--destructive", "#e06c75");
  const green = color("--chart-5", "#98c379");
  const yellow = color("--chart-4", "#e5c07b");
  const blue = color("--chart-3", "#61afef");
  const magenta = color("--primary", accentFallback);
  const cyan = color("--chart-2", "#56b6c2");

  return {
    background,
    foreground,
    cursor: foreground,
    cursorAccent: background,
    selectionBackground: color("--primary", "#777777", 0.25),
    selectionInactiveBackground: color("--muted-foreground", "#555555", 0.2),
    scrollbarSliderBackground: color("--muted-foreground", "#777777", 0.25),
    scrollbarSliderHoverBackground: color("--muted-foreground", "#777777", 0.4),
    black,
    brightBlack: muted,
    red,
    brightRed: red,
    green,
    brightGreen: green,
    yellow,
    brightYellow: yellow,
    blue,
    brightBlue: blue,
    magenta,
    brightMagenta: magenta,
    cyan,
    brightCyan: cyan,
    white: foreground,
    brightWhite: foreground,
  };
}

function terminalFontSize(): number {
  const rootFontSize = Number.parseFloat(
    window.getComputedStyle(document.documentElement).fontSize,
  );
  const safeRootFontSize = Number.isFinite(rootFontSize) ? rootFontSize : 16;
  // xterm accepts pixels, so derive them from the app's zoom-scaled root rem.
  return safeRootFontSize * TERMINAL_FONT_REM;
}

function terminalDimensions(terminal: Terminal) {
  return {
    cols: Math.min(MAX_TERMINAL_DIMENSION, Math.max(1, terminal.cols)),
    rows: Math.min(MAX_TERMINAL_DIMENSION, Math.max(1, terminal.rows)),
  };
}

function sameSession(
  left: CodeTerminalSession | null,
  right: CodeTerminalSession,
): boolean {
  return (
    left !== null &&
    left.sessionId === right.sessionId &&
    left.threadId === right.threadId &&
    codeScopesEqual(left.scope, right.scope)
  );
}

function terminateSession(session: CodeTerminalSession): Promise<void> {
  return terminateCodeTerminal({
    scope: session.scope,
    threadId: session.threadId,
    sessionId: session.sessionId,
  });
}

/** Capture the Code-route terminal shortcut before app-shell/browser handlers. */
export function useCodeTerminalShortcut(
  onToggle: () => void,
  enabled: boolean,
) {
  const onToggleRef = React.useRef(onToggle);
  const enabledRef = React.useRef(enabled);
  onToggleRef.current = onToggle;
  enabledRef.current = enabled;

  React.useLayoutEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (
        !enabledRef.current ||
        event.defaultPrevented ||
        event.key.toLowerCase() !== "j" ||
        event.altKey ||
        event.shiftKey ||
        !hasPrimaryShortcutModifier(event)
      ) {
        return;
      }

      event.preventDefault();
      event.stopImmediatePropagation();
      if (!event.repeat) onToggleRef.current();
    };

    window.addEventListener("keydown", handleKeyDown, true);
    return () => window.removeEventListener("keydown", handleKeyDown, true);
  }, []);
}

export function CodeTerminalDrawer({
  onOpenChange,
  open,
  scope,
  threadId,
}: {
  onOpenChange: (open: boolean) => void;
  open: boolean;
  scope: CodeThreadBindingScope;
  threadId: string;
}) {
  const { accentColor, isDark, themeName } = useTheme();
  const [rendererActivated, setRendererActivated] = React.useState(open);
  const [phase, setPhase] = React.useState<TerminalPhase>("idle");
  const [exitStatus, setExitStatus] = React.useState<ExitStatus | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const terminalHostRef = React.useRef<HTMLDivElement>(null);
  const terminalRef = React.useRef<Terminal | null>(null);
  const fitAddonRef = React.useRef<FitAddon | null>(null);
  const sessionRef = React.useRef<CodeTerminalSession | null>(null);
  const phaseRef = React.useRef<TerminalPhase>("idle");
  const openRef = React.useRef(open);
  const focusReturnRef = React.useRef<HTMLElement | null>(null);
  const previousOpenRef = React.useRef(false);
  const destroyedRef = React.useRef(false);
  const terminateOnOpenRef = React.useRef(false);
  const lifetimeEpochRef = React.useRef(0);
  const openAttemptRef = React.useRef(0);
  const resizeFrameRef = React.useRef<number | null>(null);
  const stdinQueueRef = React.useRef<Promise<void>>(Promise.resolve());
  const resizeQueueRef = React.useRef<Promise<void>>(Promise.resolve());
  openRef.current = open;

  const exactScope = React.useMemo<CodeThreadBindingScope>(
    () => ({
      communityId: scope.communityId,
      projectDtag: scope.projectDtag,
      repositoryIdentity: scope.repositoryIdentity,
    }),
    [scope.communityId, scope.projectDtag, scope.repositoryIdentity],
  );

  const updatePhase = React.useCallback((next: TerminalPhase) => {
    phaseRef.current = next;
    if (!destroyedRef.current) setPhase(next);
  }, []);

  const reportError = React.useCallback((cause: unknown) => {
    if (!destroyedRef.current) setError(errorMessage(cause));
  }, []);

  const queueResize = React.useCallback(
    (session: CodeTerminalSession, cols: number, rows: number) => {
      if (session.cols === cols && session.rows === rows) return;
      const resizedSession = { ...session, cols, rows };
      sessionRef.current = resizedSession;
      resizeQueueRef.current = resizeQueueRef.current
        .catch(() => {})
        .then(async () => {
          if (
            destroyedRef.current ||
            phaseRef.current !== "running" ||
            !sameSession(sessionRef.current, resizedSession)
          ) {
            return;
          }
          await resizeCodeTerminal(resizedSession);
        })
        .catch(reportError);
    },
    [reportError],
  );

  const fitTerminal = React.useCallback(() => {
    const terminal = terminalRef.current;
    const fitAddon = fitAddonRef.current;
    const host = terminalHostRef.current;
    if (
      !openRef.current ||
      terminal === null ||
      fitAddon === null ||
      host === null ||
      host.clientWidth === 0 ||
      host.clientHeight === 0
    ) {
      return;
    }

    const fontSize = terminalFontSize();
    if (terminal.options.fontSize !== fontSize) {
      terminal.options.fontSize = fontSize;
    }
    fitAddon.fit();
    const session = sessionRef.current;
    if (session !== null) {
      const dimensions = terminalDimensions(terminal);
      queueResize(session, dimensions.cols, dimensions.rows);
    }
  }, [queueResize]);

  const scheduleFit = React.useCallback(() => {
    if (resizeFrameRef.current !== null) return;
    resizeFrameRef.current = window.requestAnimationFrame(() => {
      resizeFrameRef.current = null;
      fitTerminal();
    });
  }, [fitTerminal]);

  React.useLayoutEffect(() => {
    if (open) setRendererActivated(true);
  }, [open]);

  React.useLayoutEffect(() => {
    if (!rendererActivated) return;
    const host = terminalHostRef.current;
    if (host === null) return;

    const terminal = new Terminal({
      allowTransparency: false,
      convertEol: false,
      cursorBlink: false,
      fontFamily:
        'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", monospace',
      fontSize: terminalFontSize(),
      lineHeight: 1.2,
      screenReaderMode: true,
      scrollback: 5_000,
      theme: terminalTheme(),
    });
    Terminal.strings.promptLabel = "Terminal input";
    const fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(host);
    terminal.attachCustomKeyEventHandler((event) => {
      if (event.key === "Escape") event.stopPropagation();
      return true;
    });
    terminalRef.current = terminal;
    fitAddonRef.current = fitAddon;

    const inputDisposable = terminal.onData((data) => {
      const session = sessionRef.current;
      if (session === null || phaseRef.current !== "running") return;
      const bytes = new TextEncoder().encode(data);
      for (
        let offset = 0;
        offset < bytes.length;
        offset += MAX_STDIN_CHUNK_BYTES
      ) {
        const chunk = Array.from(
          bytes.subarray(offset, offset + MAX_STDIN_CHUNK_BYTES),
        );
        stdinQueueRef.current = stdinQueueRef.current
          .catch(() => {})
          .then(async () => {
            if (
              destroyedRef.current ||
              phaseRef.current !== "running" ||
              !sameSession(sessionRef.current, session)
            ) {
              return;
            }
            await writeCodeTerminalStdin({
              scope: session.scope,
              threadId: session.threadId,
              sessionId: session.sessionId,
              data: chunk,
            });
          })
          .catch(reportError);
      }
    });
    const observer = new ResizeObserver(scheduleFit);
    observer.observe(host);
    scheduleFit();

    return () => {
      observer.disconnect();
      inputDisposable.dispose();
      if (resizeFrameRef.current !== null) {
        window.cancelAnimationFrame(resizeFrameRef.current);
        resizeFrameRef.current = null;
      }
      terminalRef.current = null;
      fitAddonRef.current = null;
      terminal.dispose();
    };
  }, [rendererActivated, reportError, scheduleFit]);

  React.useLayoutEffect(() => {
    if (!themeName) return;
    const terminal = terminalRef.current;
    if (terminal !== null) {
      terminal.options.theme = terminalTheme(isDark, accentColor);
      scheduleFit();
    }
  }, [accentColor, isDark, scheduleFit, themeName]);

  const startTerminal = React.useCallback(async () => {
    if (
      destroyedRef.current ||
      phaseRef.current === "starting" ||
      phaseRef.current === "running" ||
      phaseRef.current === "terminating"
    ) {
      return;
    }

    const attempt = openAttemptRef.current + 1;
    openAttemptRef.current = attempt;
    updatePhase("starting");
    setError(null);
    setExitStatus(null);

    await new Promise<void>((resolve) => {
      window.requestAnimationFrame(() => resolve());
    });
    if (destroyedRef.current || openAttemptRef.current !== attempt) return;
    fitTerminal();

    const terminal = terminalRef.current;
    if (terminal === null) {
      updatePhase("error");
      reportError(new Error("Terminal renderer is unavailable."));
      return;
    }

    let resolvedSession: CodeTerminalSession | null = null;
    let ended = false;
    const pendingEvents: CodeTerminalEvent[] = [];
    const consumeEvent = (event: CodeTerminalEvent) => {
      if (ended || openAttemptRef.current !== attempt) return;
      if (resolvedSession === null) {
        pendingEvents.push(event);
        return;
      }
      if (
        !codeScopesEqual(event.scope, resolvedSession.scope) ||
        event.threadId !== resolvedSession.threadId ||
        event.sessionId !== resolvedSession.sessionId
      ) {
        reportError(
          new Error("Terminal output owner did not match the session."),
        );
        return;
      }

      if (event.type === "output") {
        terminalRef.current?.write(Uint8Array.from(event.data));
        return;
      }

      ended = true;
      if (sameSession(sessionRef.current, resolvedSession)) {
        sessionRef.current = null;
      }
      setExitStatus({ exitCode: event.exitCode, signal: event.signal });
      updatePhase("exited");
    };

    try {
      const dimensions = terminalDimensions(terminal);
      const session = await openCodeTerminal(
        {
          scope: exactScope,
          threadId,
          cols: dimensions.cols,
          rows: dimensions.rows,
        },
        consumeEvent,
      );
      resolvedSession = session;
      if (
        destroyedRef.current ||
        terminateOnOpenRef.current ||
        openAttemptRef.current !== attempt
      ) {
        ended = true;
        await terminateSession(session).catch(() => {});
        return;
      }

      sessionRef.current = session;
      for (const event of pendingEvents.splice(0)) consumeEvent(event);
      if (ended) return;

      queueResize(session, dimensions.cols, dimensions.rows);
      updatePhase("running");
      if (openRef.current) terminalRef.current?.focus();
    } catch (cause) {
      if (destroyedRef.current || openAttemptRef.current !== attempt) return;
      sessionRef.current = null;
      updatePhase("error");
      reportError(cause);
    }
  }, [
    exactScope,
    fitTerminal,
    queueResize,
    reportError,
    threadId,
    updatePhase,
  ]);

  const requestTerminate = React.useCallback(async () => {
    const session = sessionRef.current;
    if (session === null || phaseRef.current !== "running") return;
    updatePhase("terminating");
    setError(null);
    try {
      await terminateSession(session);
      if (sameSession(sessionRef.current, session)) {
        sessionRef.current = null;
        setExitStatus(null);
        updatePhase("exited");
      }
    } catch (cause) {
      if (sameSession(sessionRef.current, session)) updatePhase("running");
      reportError(cause);
    }
  }, [reportError, updatePhase]);

  React.useEffect(() => {
    const epoch = lifetimeEpochRef.current + 1;
    lifetimeEpochRef.current = epoch;
    destroyedRef.current = false;
    terminateOnOpenRef.current = false;

    return () => {
      queueMicrotask(() => {
        // React Strict Mode replays effects synchronously; only a real unmount
        // leaves this epoch current when the microtask runs.
        if (lifetimeEpochRef.current !== epoch) return;
        destroyedRef.current = true;
        terminateOnOpenRef.current = true;
        openAttemptRef.current += 1;
        const session = sessionRef.current;
        sessionRef.current = null;
        if (session !== null) void terminateSession(session).catch(() => {});
      });
    };
  }, []);

  React.useEffect(() => {
    const wasOpen = previousOpenRef.current;
    previousOpenRef.current = open;
    if (open && !wasOpen) {
      focusReturnRef.current =
        document.activeElement instanceof HTMLElement
          ? document.activeElement
          : null;
      scheduleFit();
      if (phaseRef.current === "running") {
        window.requestAnimationFrame(() => {
          if (
            !destroyedRef.current &&
            openRef.current &&
            phaseRef.current === "running"
          ) {
            fitTerminal();
            terminalRef.current?.focus();
          }
        });
      } else if (phaseRef.current === "idle") {
        void startTerminal();
      }
      return;
    }
    if (!open && wasOpen) {
      const focusTarget = focusReturnRef.current;
      queueMicrotask(() => {
        if (focusTarget?.isConnected) focusTarget.focus();
      });
    }
  }, [fitTerminal, open, scheduleFit, startTerminal]);

  const statusLabel =
    phase === "starting"
      ? "Starting…"
      : phase === "running"
        ? "Running"
        : phase === "terminating"
          ? "Stopping…"
          : phase === "exited"
            ? exitStatus?.signal
              ? `Exited (${exitStatus.signal})`
              : exitStatus === null
                ? "Exited"
                : `Exited (code ${exitStatus.exitCode})`
            : phase === "error"
              ? "Unavailable"
              : "Ready";
  const canRestart =
    phase === "idle" || phase === "exited" || phase === "error";

  return (
    <section
      aria-busy={phase === "starting" || phase === "terminating"}
      aria-labelledby="code-terminal-heading"
      className="flex h-48 shrink-0 flex-col border-border/60 border-t bg-background md:h-64"
      data-state={phase}
      data-testid="code-terminal-drawer"
      hidden={!open}
    >
      <header className="flex h-9 shrink-0 items-center gap-2 border-border/60 border-b px-3">
        <SquareTerminal aria-hidden="true" className="size-3.5" />
        <h3
          className="text-balance text-xs font-semibold"
          id="code-terminal-heading"
        >
          Terminal
        </h3>
        <span className="text-2xs text-muted-foreground">{statusLabel}</span>
        <div className="flex-1" />
        {canRestart ? (
          <Button
            onClick={() => void startTerminal()}
            size="xs"
            type="button"
            variant="ghost"
          >
            <RotateCcw aria-hidden="true" />
            Start terminal
          </Button>
        ) : null}
        <AlertDialog>
          <AlertDialogTrigger asChild>
            <Button
              aria-label="Terminate terminal session"
              disabled={phase !== "running"}
              size="icon-xs"
              title="Terminate terminal session"
              type="button"
              variant="ghost"
            >
              <Square aria-hidden="true" className="fill-current" />
            </Button>
          </AlertDialogTrigger>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>Terminate terminal session?</AlertDialogTitle>
              <AlertDialogDescription className="text-pretty">
                This ends the shell and its child processes for this Code task.
                The terminal transcript will remain available in the drawer.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>Cancel</AlertDialogCancel>
              <AlertDialogAction
                className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
                onClick={() => void requestTerminate()}
              >
                Terminate session
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
        <Button
          aria-label="Hide terminal"
          onClick={() => onOpenChange(false)}
          size="icon-xs"
          title="Hide terminal"
          type="button"
          variant="ghost"
        >
          <ChevronDown aria-hidden="true" />
        </Button>
      </header>
      {error ? (
        <div
          className="border-destructive/30 border-b bg-destructive/10 px-3 py-1.5 text-xs text-destructive"
          role="alert"
        >
          {error}
        </div>
      ) : null}
      <div
        className="min-h-0 flex-1 overflow-hidden p-2 [&_.xterm]:h-full"
        data-testid="code-terminal"
        ref={terminalHostRef}
      />
    </section>
  );
}
