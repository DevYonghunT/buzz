import assert from "node:assert/strict";
import test from "node:test";

import { codeCommitDialogControls } from "./codeCommitDialogState.ts";

test("unknown commit outcome preserves editing and close without allowing resubmit", () => {
  assert.deepEqual(
    codeCommitDialogControls({
      blockedReason: "Commit outcome is unknown.",
      messageValid: true,
      submitting: false,
    }),
    { canDismiss: true, canEdit: true, canSubmit: false },
  );
});

test("only an active commit request traps dialog dismissal", () => {
  assert.deepEqual(
    codeCommitDialogControls({
      blockedReason: "Applying commit…",
      messageValid: true,
      submitting: true,
    }),
    { canDismiss: false, canEdit: false, canSubmit: false },
  );
});
