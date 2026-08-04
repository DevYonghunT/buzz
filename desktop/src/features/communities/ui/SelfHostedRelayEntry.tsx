import React from "react";
import { useTranslation } from "react-i18next";

import { InviteRedeemForm } from "@/features/onboarding/ui/InviteRedeemForm";
import { Card } from "@/shared/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/shared/ui/dialog";

/**
 * Onboarding entry for a school that runs its own relay.
 *
 * **This adds no connection path.** Submitting a bare relay URL already works:
 * `InviteRedeemForm`'s `canSubmit` is true on `normalizedRelayUrl` alone, and
 * `WelcomeSetup`'s `startConnection` takes it from there. What was missing was
 * a door — every labelled route either opened hosted sign-in or asked for an
 * invite code, and an invite can only be minted by an owner or admin, so a
 * brand-new relay has nobody to ask. See
 * `docs/schoolx-2/SELF_HOSTED_ONBOARDING.md` §1–§2.
 *
 * Split into three pieces because the two doors sit at different depths:
 * the card stands beside other cards and can own its dialog, while the link
 * sits *inside* the hosted sign-in dialog and cannot — a dialog rendered
 * under another dialog's overlay is blurred out and swallows no clicks. So
 * the caller owns {@link SelfHostedRelayDialog} at the top level and the link
 * only asks for it to open.
 *
 * All of this lives in a SchoolX-owned file on purpose: the callers are
 * upstream code that upstream keeps changing (#2738, #2862), so they get a
 * render line or two and nothing else (§3).
 */

/** Shared copy + form. Controlled, so whoever renders it decides the depth. */
export function SelfHostedRelayDialog({
  onConnect,
  onOpenChange,
  open,
}: {
  /** Relay URL submit. Takes `WelcomeSetup`'s `startConnection` as-is. */
  onConnect: (relayWsUrl: string) => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const { t } = useTranslation();

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent data-testid="self-hosted-relay-dialog">
        <DialogTitle>{t("app.selfHostedRelay.dialogTitle")}</DialogTitle>
        <DialogDescription>
          {t("app.selfHostedRelay.dialogDescription")}
        </DialogDescription>
        {/*
          `variant="default"` matters: under `add-community` the
          `normalizedRelayUrl` memo takes a different branch (`hasInviteRelay`)
          and a bare URL stops being submittable.

          `onRedeem` is required but unused here — pasting an invite code into
          this dialog is not what it is for, and the existing join screens
          already handle that case.
        */}
        <InviteRedeemForm
          error={null}
          isRedeeming={false}
          onCancel={() => onOpenChange(false)}
          onConnect={(relayWsUrl) => {
            onOpenChange(false);
            onConnect(relayWsUrl);
          }}
          onRedeem={() => {}}
          placeholder={t("app.selfHostedRelay.placeholder")}
          variant="default"
        />
      </DialogContent>
    </Dialog>
  );
}

/**
 * One quiet line for someone already inside the hosted sign-in dialog.
 *
 * Renders no dialog of its own — see the note above. `onSelect` is expected to
 * close the hosted dialog first and then open {@link SelfHostedRelayDialog}
 * from a level where nothing covers it.
 */
export function SelfHostedRelayLink({ onSelect }: { onSelect: () => void }) {
  const { t } = useTranslation();

  return (
    <button
      className="mt-3 text-xs underline underline-offset-2 hover:no-underline"
      data-testid="self-hosted-relay-link"
      onClick={onSelect}
      type="button"
    >
      {t("app.selfHostedRelay.link")}
    </button>
  );
}

/** A choice sitting among the other onboarding cards, with its own dialog. */
export function SelfHostedRelayEntry({
  cardClassName,
  onConnect,
}: {
  /**
   * Sizing for the card, so it matches the cards it sits beside.
   *
   * Passed in rather than imported: the sibling cards' class lives in an
   * unexported const inside `WelcomeSetup.tsx`, and importing from upstream
   * files is exactly the coupling this component exists to avoid.
   */
  cardClassName?: string;
  onConnect: (relayWsUrl: string) => void;
}) {
  const { t } = useTranslation();
  const [isOpen, setIsOpen] = React.useState(false);

  return (
    <>
      <Card asChild className={cardClassName} variant="textured">
        <button
          className="flex w-full flex-col items-start gap-1 text-left"
          data-testid="self-hosted-relay-card"
          onClick={() => setIsOpen(true)}
          type="button"
        >
          <span className="font-medium">
            {t("app.selfHostedRelay.cardTitle")}
          </span>
          <span className="text-sm text-foreground/70">
            {t("app.selfHostedRelay.cardDescription")}
          </span>
        </button>
      </Card>
      <SelfHostedRelayDialog
        onConnect={onConnect}
        onOpenChange={setIsOpen}
        open={isOpen}
      />
    </>
  );
}
