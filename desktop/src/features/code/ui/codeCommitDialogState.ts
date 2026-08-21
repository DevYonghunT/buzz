export function codeCommitDialogControls({
  blockedReason,
  messageValid,
  submitting,
}: {
  blockedReason: string | null;
  messageValid: boolean;
  submitting: boolean;
}) {
  return {
    canDismiss: !submitting,
    canEdit: !submitting,
    canSubmit: !submitting && blockedReason === null && messageValid,
  };
}
