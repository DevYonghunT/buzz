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
 * Why this lives in its own file: the caller (`WelcomeSetup.tsx`) is upstream
 * code SchoolX has never modified, and upstream keeps changing it (#2738,
 * #2862). Keeping all state and markup here leaves the caller a render line
 * or two, so an upstream sync has almost nothing to conflict with (§3).
 */
export function SelfHostedRelayEntry({
  onConnect,
  variant = "card",
}: {
  /** Relay URL submit. Takes `WelcomeSetup`'s `startConnection` as-is. */
  onConnect: (relayWsUrl: string) => void;
  /**
   * `card` — a choice sitting among the other onboarding cards.
   * `link` — one quiet line for someone already inside the hosted dialog.
   * That is the dead end people actually reach, so it needs a way out, but
   * it should not compete with the hosted flow it sits on top of.
   */
  variant?: "card" | "link";
}) {
  const { t } = useTranslation();
  const [isOpen, setIsOpen] = React.useState(false);

  return (
    <>
      {variant === "card" ? (
        <Card asChild variant="textured">
          <button
            className="flex w-full flex-col items-start gap-1 p-6 text-left"
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
      ) : (
        <button
          className="text-xs underline underline-offset-2 hover:no-underline"
          data-testid="self-hosted-relay-link"
          onClick={() => setIsOpen(true)}
          type="button"
        >
          {t("app.selfHostedRelay.link")}
        </button>
      )}

      <Dialog onOpenChange={setIsOpen} open={isOpen}>
        <DialogContent data-testid="self-hosted-relay-dialog">
          <DialogTitle>{t("app.selfHostedRelay.dialogTitle")}</DialogTitle>
          <DialogDescription>
            {t("app.selfHostedRelay.dialogDescription")}
          </DialogDescription>
          {/*
            `variant="default"` matters: under `add-community` the
            `normalizedRelayUrl` memo takes a different branch
            (`hasInviteRelay`) and a bare URL stops being submittable.

            `onRedeem` is required but unused here — pasting an invite code
            into this dialog is not what it is for, and the existing join
            screens already handle that case.
          */}
          <InviteRedeemForm
            error={null}
            isRedeeming={false}
            onCancel={() => setIsOpen(false)}
            onConnect={(relayWsUrl) => {
              setIsOpen(false);
              onConnect(relayWsUrl);
            }}
            onRedeem={() => {}}
            placeholder={t("app.selfHostedRelay.placeholder")}
            variant="default"
          />
        </DialogContent>
      </Dialog>
    </>
  );
}
