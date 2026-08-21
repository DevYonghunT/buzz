import { LoaderCircle, Send, Square } from "lucide-react";
import * as React from "react";

import { Button } from "@/shared/ui/button";
import { Textarea } from "@/shared/ui/textarea";

export function CodeComposer({
  active,
  canInterrupt,
  disabled,
  disabledReason,
  onInterrupt,
  onSubmit,
}: {
  active: boolean;
  canInterrupt: boolean;
  disabled: boolean;
  disabledReason: string | null;
  onInterrupt: () => Promise<void>;
  onSubmit: (prompt: string) => Promise<boolean>;
}) {
  const [prompt, setPrompt] = React.useState("");
  const [submitting, setSubmitting] = React.useState(false);
  const [interrupting, setInterrupting] = React.useState(false);

  const submit = React.useCallback(async () => {
    const nextPrompt = prompt.trim();
    if (!nextPrompt || disabled || submitting) return;
    setSubmitting(true);
    try {
      if (await onSubmit(nextPrompt)) setPrompt("");
    } finally {
      setSubmitting(false);
    }
  }, [disabled, onSubmit, prompt, submitting]);

  return (
    <div className="border-border/60 border-t bg-background/95 p-3 backdrop-blur">
      <div className="mx-auto max-w-3xl rounded-xl border border-input/50 bg-background p-2 shadow-xs focus-within:ring-1 focus-within:ring-ring">
        <Textarea
          aria-describedby={
            disabledReason ? "code-composer-disabled-reason" : undefined
          }
          aria-label={active ? "Steer active Code task" : "Message Code task"}
          className="min-h-16 resize-none border-0 bg-transparent p-2 shadow-none focus-visible:ring-0 md:text-base"
          disabled={disabled}
          onChange={(event) => setPrompt(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape" && prompt) {
              event.preventDefault();
              setPrompt("");
              return;
            }
            if (event.key === "Enter" && event.metaKey) {
              event.preventDefault();
              void submit();
            }
          }}
          placeholder={
            active
              ? "Add direction to the active turn…"
              : "Ask Codex to work in this repository…"
          }
          value={prompt}
        />
        {disabledReason ? (
          <p
            className="px-2 pb-1 text-pretty text-2xs text-destructive"
            id="code-composer-disabled-reason"
            role="status"
          >
            {disabledReason}
          </p>
        ) : null}
        <div className="flex items-center justify-between gap-2 px-1 pb-1">
          <span className="text-2xs text-muted-foreground">
            ⌘↵ to {active ? "steer" : "send"} · Esc to clear
          </span>
          <div className="flex items-center gap-1.5">
            {active ? (
              <Button
                disabled={!canInterrupt || interrupting}
                onClick={() => {
                  setInterrupting(true);
                  void onInterrupt().finally(() => setInterrupting(false));
                }}
                size="sm"
                variant="outline"
              >
                {interrupting ? (
                  <LoaderCircle className="animate-spin motion-reduce:animate-none" />
                ) : (
                  <Square />
                )}
                Stop
              </Button>
            ) : null}
            <Button
              aria-label={active ? "Steer active turn" : "Send prompt"}
              disabled={disabled || submitting || !prompt.trim()}
              onClick={() => void submit()}
              size="sm"
            >
              {submitting ? (
                <LoaderCircle className="animate-spin motion-reduce:animate-none" />
              ) : (
                <Send />
              )}
              {active ? "Steer" : "Send"}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
