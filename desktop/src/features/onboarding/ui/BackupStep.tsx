import { Check, Copy, Eye, EyeOff, Info, ShieldCheck } from "lucide-react";
import { useReducedMotion } from "motion/react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { getNsec } from "@/shared/api/tauriIdentity";
import type { IdentityStorage } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { writeTextToClipboard } from "@/shared/lib/clipboard";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { SchoolXMark } from "@/shared/ui/schoolx-brand/SchoolXMark";
import { Spinner } from "@/shared/ui/spinner";
import {
  ONBOARDING_PRIMARY_CTA_CLASS,
  ONBOARDING_SECONDARY_CTA_CLASS,
} from "./OnboardingChrome";
import { OnboardingFooter } from "./OnboardingFooter";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import { ONBOARDING_KEY_TEXT_CLASS } from "./NsecMaskedDisplay";

/**
 * How long the "Creating your identity key" loader holds the stage before the
 * finished state fades in. Purely perceptual — the key already exists; the
 * pause sells the creation moment.
 */
const INTRO_HOLD_MS = 1400;

/**
 * The creation moment should only be sold once per app session. Module-level
 * so remounts (e.g. navigating Back and returning to this step) skip the fake
 * hold and show the finished state instantly.
 */
let introPlayed = false;

const REVEAL_ANIMATION_CLASS =
  "animate-in fade-in duration-700 motion-reduce:animate-none";

const BACKUP_OPTION_CLASS =
  "flex min-h-48 w-full flex-col items-start justify-start px-6 py-5 text-left text-foreground";

/** Viewing the key never blocks onboarding — Next is always actionable. */
export function backupNextDisabled(): boolean {
  return false;
}

type BackupStepProps = {
  direction: OnboardingTransitionDirection;
  identityStorage?: IdentityStorage;
  onBack: () => void;
  onNext: () => void;
  onOpenPasswordBackup: () => void;
  onShowOptions: () => void;
  optionsExpanded: boolean;
  returningFromSecurity: boolean;
};

/**
 * Onboarding identity-key step — shows the freshly created key, then opens a
 * dark backup-options state. Copy fetches the raw key only after an explicit
 * click; password backup opens the separate security flow. Neither method
 * blocks Next.
 */
export function BackupStep({
  direction,
  identityStorage,
  onBack,
  onNext,
  onOpenPasswordBackup,
  onShowOptions,
  optionsExpanded,
  returningFromSecurity,
}: BackupStepProps) {
  const reduceMotion = useReducedMotion() ?? false;
  const [created, setCreated] = React.useState(introPlayed || reduceMotion);
  const [copyState, setCopyState] = React.useState<
    "idle" | "copying" | "copied"
  >("idle");
  const [copyError, setCopyError] = React.useState<string | null>(null);
  const [nsec, setNsec] = React.useState<string | null>(null);
  const [isRevealed, setIsRevealed] = React.useState(false);
  const cancelledRef = React.useRef(false);
  const copiedTimerRef = React.useRef<number | null>(null);
  const { t } = useTranslation();

  React.useEffect(() => {
    if (introPlayed) return;
    if (reduceMotion) {
      introPlayed = true;
      setCreated(true);
      return;
    }
    const timer = window.setTimeout(() => {
      introPlayed = true;
      setCreated(true);
    }, INTRO_HOLD_MS);
    return () => window.clearTimeout(timer);
  }, [reduceMotion]);

  React.useEffect(() => {
    cancelledRef.current = false;
    return () => {
      // Back-during-fetch: cancel any in-flight setState calls and clear the
      // nsec from memory on unmount (backup step is only on the fresh-key path).
      cancelledRef.current = true;
      setNsec(null);
      if (copiedTimerRef.current !== null)
        window.clearTimeout(copiedTimerRef.current);
    };
  }, []);

  const copyKeyToClipboard = React.useCallback(async () => {
    setCopyState("copying");
    setCopyError(null);
    try {
      const value = nsec ?? (await getNsec());
      await writeTextToClipboard(value);
      if (cancelledRef.current) return;
      setCopyState("copied");
      if (copiedTimerRef.current !== null)
        window.clearTimeout(copiedTimerRef.current);
      copiedTimerRef.current = window.setTimeout(() => {
        if (!cancelledRef.current) setCopyState("idle");
      }, 2000);
    } catch (err) {
      if (cancelledRef.current) return;
      setCopyState("idle");
      setCopyError(
        err instanceof Error ? err.message : "Failed to retrieve private key.",
      );
    }
  }, [nsec]);

  const toggleReveal = React.useCallback(async () => {
    if (isRevealed) {
      setIsRevealed(false);
      return;
    }
    setCopyError(null);
    try {
      // The raw key enters the DOM only after this explicit reveal action.
      const value = nsec ?? (await getNsec());
      if (cancelledRef.current) return;
      setNsec(value);
      setIsRevealed(true);
    } catch (err) {
      if (cancelledRef.current) return;
      setCopyError(
        err instanceof Error ? err.message : "Failed to retrieve private key.",
      );
    }
  }, [isRevealed, nsec]);

  // Fixed-length decorative mask (nsec keys are 63 chars) so no key material
  // is fetched just to render the blurred row. Bullets are joined with a
  // zero-width space: WebKit won't line-break a run of U+2022 without an
  // explicit break opportunity, so the masked row would overflow otherwise.
  const maskedKey = React.useMemo(
    () => Array.from({ length: nsec?.length ?? 63 }, () => "•").join("\u200b"),
    [nsec],
  );
  const storageDescription =
    identityStorage === "system-keyring"
      ? t("app.onboarding.backup.storage.keychain")
      : identityStorage === "local-file"
        ? t("app.onboarding.backup.storage.fileFallback")
        : t("app.onboarding.backup.storage.device");
  const storageTitle =
    identityStorage === "system-keyring"
      ? t("app.onboarding.backup.storageTitle.keychain")
      : identityStorage === "local-file"
        ? t("app.onboarding.backup.storageTitle.fileFallback")
        : t("app.onboarding.backup.storageTitle.device");
  const introStorageDescription =
    identityStorage === "system-keyring"
      ? t("app.onboarding.backup.introStorage.keychain")
      : identityStorage === "local-file"
        ? t("app.onboarding.backup.introStorage.fileFallback")
        : t("app.onboarding.backup.introStorage.device");

  if (optionsExpanded) {
    return (
      <OnboardingSlideTransition
        className="flex min-h-0 w-full flex-col items-center"
        data-testid="onboarding-page-backup-options"
        direction={direction}
        effect={direction === "forward" ? "mask-reveal-up" : "line-slide"}
        transitionKey={`backup-options-${direction}`}
      >
        <div className="flex w-full max-w-140 shrink-0 flex-col text-center">
          <h1 className="text-balance text-title font-normal text-foreground">
            {t("app.onboarding.backup.options.title")}
          </h1>
          <p className="mt-5 text-pretty text-sm leading-6 text-foreground/75">
            {t("app.onboarding.backup.options.description")}
          </p>
        </div>

        <div className="flex w-full max-w-260 flex-1 flex-col justify-center py-10">
          <div
            className="grid w-full grid-cols-1 gap-5 md:grid-cols-2 lg:grid-cols-3"
            data-testid="backup-options"
          >
            <div
              className={cn(BACKUP_OPTION_CLASS, "md:col-span-2 lg:col-span-1")}
              data-testid="backup-option-panel"
            >
              <span className="text-lg font-medium">{storageTitle}</span>
              <span className="mt-3 block text-sm leading-6 text-foreground/65">
                {storageDescription}
              </span>
            </div>

            <div
              className={BACKUP_OPTION_CLASS}
              data-testid="backup-option-panel"
            >
              <span className="text-lg font-medium">
                Saved in your password manager
              </span>
              <span className="mt-3 block text-sm leading-6 text-foreground/65">
                Copy your identity key, then save it in a password manager like
                1Password.
              </span>
              <Button
                className={cn(
                  ONBOARDING_SECONDARY_CTA_CLASS,
                  "mt-5 w-fit gap-2 px-5",
                )}
                data-testid="backup-copy-key"
                disabled={copyState === "copying"}
                onClick={() => void copyKeyToClipboard()}
                type="button"
                variant="ghost"
              >
                {copyState === "copying" ? (
                  <Spinner className="h-4 w-4 border-2" />
                ) : copyState === "copied" ? (
                  <Check className="h-4 w-4" aria-hidden="true" />
                ) : (
                  <Copy className="h-4 w-4" aria-hidden="true" />
                )}
                {copyState === "copying"
                  ? "Copying…"
                  : copyState === "copied"
                    ? t("app.onboarding.backup.copied")
                    : t("app.onboarding.backup.copy")}
              </Button>
            </div>

            <div
              className={BACKUP_OPTION_CLASS}
              data-testid="backup-option-panel"
            >
              <span className="text-lg font-medium">
                Locked in a backup file
              </span>
              <span className="mt-3 block text-sm leading-6 text-foreground/65">
                Create a backup file and choose a password you can remember.
                You’ll need both to restore your account.
              </span>
              <Button
                className={cn(
                  ONBOARDING_SECONDARY_CTA_CLASS,
                  "mt-5 w-fit gap-2 px-5",
                )}
                data-testid="backup-option-password"
                onClick={onOpenPasswordBackup}
                type="button"
                variant="ghost"
              >
                <ShieldCheck className="h-5 w-5" aria-hidden="true" />
                Create locked backup
              </Button>
            </div>
          </div>

          {copyError ? (
            <p
              className="mt-4 text-center text-sm text-destructive"
              data-testid="backup-copy-error"
            >
              Could not retrieve your private key: {copyError}. You can continue
              and find it later in Settings &gt; Profile &gt; Identity.
            </p>
          ) : null}
        </div>
      </OnboardingSlideTransition>
    );
  }

  return (
    <OnboardingSlideTransition
      className="flex min-h-0 w-full flex-col items-center"
      data-testid="onboarding-page-backup"
      direction={direction}
      effect={returningFromSecurity ? "mask-reveal-down" : "line-slide"}
      transitionKey={`backup-${direction}-${returningFromSecurity ? "down" : "line"}`}
    >
      <div className="flex w-full max-w-[500px] shrink-0 flex-col text-center">
        {/* Plain string concat: cn()'s tailwind-merge misreads the custom
            text-title size token as conflicting with text-foreground. */}
        <h1
          className={`text-balance text-title font-normal text-foreground ${REVEAL_ANIMATION_CLASS}`}
          key={created ? "created" : "creating"}
        >
          {created
            ? t("app.onboarding.backup.titleCreated")
            : t("app.onboarding.backup.titleCreating")}
        </h1>
        {created ? (
          <p
            className={cn(
              "mt-5 text-pretty text-sm leading-6 text-foreground/80",
              REVEAL_ANIMATION_CLASS,
            )}
          >
            {introStorageDescription}
            {t("app.onboarding.backup.introContinue")}
            <button
              className="rounded-sm font-medium underline decoration-foreground/40 underline-offset-4 transition-colors hover:decoration-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
              data-testid="backup-options-link"
              onClick={onShowOptions}
              type="button"
            >
              {t("app.onboarding.backup.optionsLink")}
            </button>
            {t("app.onboarding.backup.introRestore")}
          </p>
        ) : null}
      </div>

      {!created ? (
        <div
          className="flex w-full flex-1 items-center justify-center py-10"
          data-testid="backup-intro-logo"
        >
          <SchoolXMark className="size-20" decorative />
        </div>
      ) : (
        <div
          className={cn(
            "flex w-full max-w-[1040px] flex-1 flex-col justify-center py-10",
            REVEAL_ANIMATION_CLASS,
          )}
        >
          <div className="w-full">
            <Card className="px-8 py-6" variant="textured">
              <div className="mx-auto flex w-full min-w-0 max-w-[832px] items-center gap-4">
                <div className="min-w-0 flex-1">
                  <p
                    className={cn(
                      ONBOARDING_KEY_TEXT_CLASS,
                      isRevealed && nsec
                        ? "select-text"
                        : "select-none blur-[4px]",
                    )}
                    data-testid="backup-key-value"
                  >
                    {isRevealed && nsec ? nsec : maskedKey}
                  </p>
                </div>
                <Button
                  aria-label={
                    isRevealed
                      ? t("app.onboarding.backup.hideKey")
                      : t("app.onboarding.backup.revealKey")
                  }
                  className="h-10 w-10 shrink-0 text-muted-foreground hover:text-foreground"
                  data-testid="backup-key-reveal-toggle"
                  onClick={() => void toggleReveal()}
                  size="icon"
                  type="button"
                  variant="ghost"
                >
                  {isRevealed ? (
                    <EyeOff className="h-6 w-6" aria-hidden="true" />
                  ) : (
                    <Eye className="h-6 w-6" aria-hidden="true" />
                  )}
                </Button>
              </div>
            </Card>

            {copyError ? (
              <p
                className="mt-4 text-center text-sm text-destructive"
                data-testid="backup-copy-error"
              >
                Could not retrieve your private key: {copyError}. You can
                continue and find it later in Settings &gt; Profile &gt;
                Identity.
              </p>
            ) : null}

            <p className="mx-auto mt-5 flex max-w-[440px] items-start justify-center gap-1.5 text-center text-xs leading-5 text-[var(--buzz-onboarding-backup-ink)]">
              <Info className="mt-0.5 h-3.5 w-3.5 shrink-0" />
              <span>{t("app.onboarding.backup.neverShare")}</span>
            </p>
          </div>
        </div>
      )}

      {created ? (
        <OnboardingFooter className={REVEAL_ANIMATION_CLASS}>
          <Button
            className={ONBOARDING_PRIMARY_CTA_CLASS}
            data-testid="onboarding-next"
            disabled={backupNextDisabled()}
            onClick={onNext}
            type="button"
          >
            Next
          </Button>

          <Button
            className={ONBOARDING_SECONDARY_CTA_CLASS}
            data-testid="onboarding-back"
            onClick={onBack}
            type="button"
            variant="ghost"
          >
            Back
          </Button>
        </OnboardingFooter>
      ) : null}
    </OnboardingSlideTransition>
  );
}
