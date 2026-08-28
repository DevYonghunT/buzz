import * as React from "react";
import type { QueryClient } from "@tanstack/react-query";
import { ArrowUp } from "lucide-react";
import { motion, useReducedMotion } from "motion/react";
import { useTranslation } from "react-i18next";

import { requiredCredentialEnvKeys } from "@/features/agents/ui/agentConfigOptions";
import { getBakedBuildEnv } from "@/shared/api/tauri";
import {
  getIdentity,
  importIdentity,
  persistCurrentIdentity,
} from "@/shared/api/tauriIdentity";
import type { IdentityStorage } from "@/shared/api/types";
import { bakedAgentConfigIsComplete } from "./bakedOnboardingSkip";
import { Button } from "@/shared/ui/button";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import { BackupStep } from "./BackupStep";
import { DefaultConfigStep } from "./DefaultConfigStep";
import { DownloadKeyStep } from "./DownloadKeyStep";
import {
  backupSessionToPasswordEntry,
  resetEncryptedBackupSession,
  useEncryptedBackupSession,
} from "./EncryptedBackupCreator";
import { IdentityKeyHelpDialog } from "./IdentityKeyHelpDialog";
import {
  NostrKeyImportForm,
  type NostrKeyImportStage,
} from "./NostrKeyImportForm";
import {
  ONBOARDING_LANDING_CTA_CLASS,
  ONBOARDING_SECONDARY_CTA_CLASS,
  OnboardingChrome,
} from "./OnboardingChrome";
import { OnboardingFooterProvider } from "./OnboardingFooter";
import { OnboardingSlideTransition } from "./OnboardingSlideTransition";
import { SetupStep } from "./SetupStep";
import { SchoolXOnboardingBrand } from "./SchoolXOnboardingBrand";

export type MachineOnboardingPage =
  | "identity"
  | "key-import"
  | "backup"
  | "setup"
  | "config";

type BackupSubview = "ready" | "options" | "password";

/** A pending navigation the parent should execute after RouterProvider mounts. */
export type PostOnboardingNavigation = {
  to: string;
  search?: Record<string, string>;
};

export function MachineOnboardingFlow({
  complete,
  continueWithIdentity,
  identityLost,
  initialPage,
  queryClient,
  navigateAfterComplete,
}: {
  complete: (pubkey?: string) => void;
  continueWithIdentity: (pubkey: string) => void;
  identityLost: boolean;
  initialPage?: MachineOnboardingPage;
  queryClient: QueryClient;
  /**
   * Called when the user finishes onboarding and requests navigation to a
   * specific route (e.g. Settings → Agents). The parent owns the RouterProvider,
   * so navigation must be deferred to it — calling router.navigate() here races
   * with RouterProvider mounting.
   */
  navigateAfterComplete?: (nav: PostOnboardingNavigation) => void;
}) {
  const { t } = useTranslation();
  const [page, setPage] = React.useState<MachineOnboardingPage>(
    identityLost ? "key-import" : (initialPage ?? "identity"),
  );
  const [error, setError] = React.useState<string | null>(null);
  const [isPending, setIsPending] = React.useState(false);
  const [identityWasImported, setIdentityWasImported] = React.useState(false);
  const [keyImportStage, setKeyImportStage] =
    React.useState<NostrKeyImportStage>("key-entry");
  const [selectedPubkey, setSelectedPubkey] = React.useState<string | null>(
    null,
  );
  const [identityStorage, setIdentityStorage] = React.useState<
    IdentityStorage | undefined
  >();
  const [readyRuntimeIds, setReadyRuntimeIds] = React.useState<string[]>([]);
  const [backupSubview, setBackupSubview] =
    React.useState<BackupSubview>("ready");
  const [backupDirection, setBackupDirection] = React.useState<
    "forward" | "backward"
  >("forward");
  const [returningFromSecurity, setReturningFromSecurity] =
    React.useState(false);
  // Owned here so switching between the yellow onboarding view and the dark
  // security subview keeps the backup password and test progress.
  const backupSession = useEncryptedBackupSession();
  const reduceMotion = useReducedMotion() ?? false;
  const isSecuritySubview = page === "backup" && backupSubview !== "ready";
  const handleReadyRuntimeIdsChange = React.useCallback(
    (runtimeIds: readonly string[]) => {
      setReadyRuntimeIds(Array.from(new Set(runtimeIds)));
    },
    [],
  );

  // A build with provider, model, and credentials baked in has nothing to ask
  // on the harness and model steps, so teammates go straight from their key to
  // the app. Everything stays editable later in Settings → Agents.
  const [skipAgentSetup, setSkipAgentSetup] = React.useState(false);
  React.useEffect(() => {
    let unmounted = false;
    void getBakedBuildEnv()
      .then((bakedEnv) => {
        if (unmounted) return;
        setSkipAgentSetup(
          bakedAgentConfigIsComplete(bakedEnv, (provider) =>
            requiredCredentialEnvKeys("buzz-agent", provider),
          ),
        );
      })
      // Never let a probe failure strand the user past configuration: on error
      // the steps stay, which is the OSS-build behaviour.
      .catch(() => undefined);
    return () => {
      unmounted = true;
    };
  }, []);

  /** Where to go once the user is done with their key. */
  const afterIdentityPage = React.useCallback(
    (pubkey?: string) => {
      if (skipAgentSetup) {
        complete(pubkey);
        return;
      }
      setPage("setup");
    },
    [complete, skipAgentSetup],
  );

  const loadDeviceIdentity = React.useCallback(async () => {
    setIsPending(true);
    setError(null);
    try {
      const identity = await getIdentity();
      queryClient.setQueryData(["identity"], identity);
      setSelectedPubkey(identity.pubkey);
      setIdentityStorage(identity.storage);
      setBackupDirection("forward");
      setReturningFromSecurity(false);
      setBackupSubview("ready");
      setPage("backup");
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Failed to load identity",
      );
    } finally {
      setIsPending(false);
    }
  }, [queryClient]);

  const replaceLostIdentity = React.useCallback(async () => {
    const confirmed = window.confirm(
      "This will create a new identity and abandon your previous key. This cannot be undone. Continue?",
    );
    if (!confirmed) return;

    setIsPending(true);
    setError(null);
    try {
      const identity = await persistCurrentIdentity();
      queryClient.setQueryData(["identity"], identity);
      setSelectedPubkey(identity.pubkey);
      setIdentityStorage(identity.storage);
      setBackupDirection("forward");
      setReturningFromSecurity(false);
      setBackupSubview("ready");
      setPage("backup");
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Failed to save identity",
      );
    } finally {
      setIsPending(false);
    }
  }, [queryClient]);

  const importExistingIdentity = React.useCallback(
    async (nsec: string, password?: string) => {
      const identity = await importIdentity(nsec, password);
      continueWithIdentity(identity.pubkey);
      queryClient.setQueryData(["identity"], identity);
      setIdentityWasImported(true);
      setSelectedPubkey(identity.pubkey);
      afterIdentityPage(identity.pubkey);
    },
    [afterIdentityPage, continueWithIdentity, queryClient],
  );

  return (
    <div
      className={`buzz-onboarding-neutral-theme buzz-startup-shell flex max-h-dvh items-start justify-center overflow-x-hidden overflow-y-auto px-4 text-foreground ${
        isSecuritySubview ? "buzz-onboarding-security-theme" : ""
      } ${
        page === "identity"
          ? "buzz-onboarding-welcome py-8"
          : "pb-28 pt-[106px]"
      }`}
      data-testid="machine-onboarding-gate"
    >
      <StartupWindowDragRegion />
      {isSecuritySubview ? (
        <div className="fixed inset-x-0 top-8 z-20 flex justify-center px-6">
          <Button
            className={`${ONBOARDING_SECONDARY_CTA_CLASS} gap-2 px-5`}
            data-testid="backup-return-to-onboarding"
            onClick={() => {
              setBackupDirection("backward");
              setReturningFromSecurity(true);
              setBackupSubview("ready");
            }}
            type="button"
            variant="ghost"
          >
            <ArrowUp className="h-4 w-4" aria-hidden="true" />
            Return to onboarding
          </Button>
        </div>
      ) : page !== "identity" ? (
        <OnboardingChrome
          current={page === "config" ? 4 : page === "setup" ? 3 : 2}
        />
      ) : null}
      <OnboardingFooterProvider>
        <div
          className={`relative flex w-full max-w-[1040px] flex-col items-center text-center ${
            page === "identity" ? "my-auto" : "buzz-onboarding-step-frame"
          }`}
        >
          {page === "identity" ? (
            <OnboardingSlideTransition
              className="flex w-full max-w-[720px] flex-col items-center text-center"
              direction="forward"
              effect="mask-reveal-up"
              transitionKey="machine-identity"
            >
              <SchoolXOnboardingBrand
                productName={t("app.productName")}
                variant="hero"
              />
              <p className="mt-5 max-w-[560px] text-pretty text-center text-2xl font-normal leading-tight text-foreground/80">
                {t("app.onboarding.landing.taglineTop")}
                <br />
                {t("app.onboarding.landing.taglineBottom")}
              </p>
              {error ? (
                <p className="mt-4 text-sm text-destructive">{error}</p>
              ) : null}
              <div className="mt-10 flex flex-col items-center gap-3">
                <Button
                  className={ONBOARDING_LANDING_CTA_CLASS}
                  disabled={isPending}
                  onClick={() => void loadDeviceIdentity()}
                  type="button"
                >
                  {isPending
                    ? t("app.onboarding.landing.loading")
                    : t("app.onboarding.landing.continueWithDeviceIdentity")}
                </Button>
                <Button
                  className={`${ONBOARDING_SECONDARY_CTA_CLASS} px-5`}
                  disabled={isPending}
                  onClick={() => {
                    setKeyImportStage("key-entry");
                    setPage("key-import");
                  }}
                  type="button"
                  variant="ghost"
                >
                  {t("app.onboarding.landing.useDifferentKey")}
                </Button>
              </div>
              <IdentityKeyHelpDialog />
            </OnboardingSlideTransition>
          ) : page === "key-import" ? (
            <OnboardingSlideTransition
              className="flex min-h-[calc(100dvh-13.25rem)] w-full max-w-[837px] flex-col items-center text-center"
              direction="forward"
              effect="fade"
              transitionKey="machine-key-import"
            >
              <motion.div
                animate={{ opacity: 1, y: 0 }}
                className="shrink-0"
                initial={reduceMotion ? false : { opacity: 0, y: 10 }}
                key={keyImportStage}
                transition={{
                  duration: reduceMotion ? 0 : 0.3,
                  ease: "easeOut",
                }}
              >
                <h1 className="text-balance text-title font-normal text-foreground">
                  {keyImportStage === "backup-password"
                    ? "Unlock your account"
                    : identityLost
                      ? "Re-import your key"
                      : "Enter your private key"}
                </h1>
                <p className="mt-5 max-w-[440px] text-pretty text-sm leading-6 text-foreground/80">
                  {keyImportStage === "backup-password"
                    ? "Enter your backup password to unlock your key and restore your identity."
                    : identityLost
                      ? "Your identity is no longer in the system keyring. Re-import your nsec to restore it."
                      : "If you already have a SchoolX account, enter your private key below to get started."}
                </p>
              </motion.div>
              <div className="buzz-onboarding-key-import-position w-full">
                <NostrKeyImportForm
                  backLabel={identityLost ? "Start new identity" : "Back"}
                  onBack={
                    identityLost
                      ? () => void replaceLostIdentity()
                      : () => setPage("identity")
                  }
                  onImport={importExistingIdentity}
                  onStageChange={setKeyImportStage}
                  variant="spotlight"
                />
              </div>
            </OnboardingSlideTransition>
          ) : page === "backup" ? (
            backupSubview === "password" ? (
              <DownloadKeyStep
                direction={backupDirection}
                onBack={() => {
                  resetEncryptedBackupSession(backupSession);
                  setBackupDirection("backward");
                  setReturningFromSecurity(false);
                  setBackupSubview("options");
                }}
                session={backupSession}
              />
            ) : (
              <BackupStep
                direction={backupDirection}
                identityStorage={identityStorage}
                onBack={() => setPage("identity")}
                onNext={() => afterIdentityPage(selectedPubkey ?? undefined)}
                onOpenPasswordBackup={() => {
                  resetEncryptedBackupSession(backupSession);
                  setBackupDirection("forward");
                  setReturningFromSecurity(false);
                  setBackupSubview("password");
                }}
                onShowOptions={() => {
                  setBackupDirection("forward");
                  setReturningFromSecurity(false);
                  setBackupSubview("options");
                }}
                optionsExpanded={backupSubview === "options"}
                returningFromSecurity={returningFromSecurity}
              />
            )
          ) : page === "setup" ? (
            <SetupStep
              actions={{
                // Device-identity users return to whichever backup subview they
                // used to reach setup; imported keys skip backup entirely.
                back: () => {
                  if (identityWasImported) {
                    setKeyImportStage("key-entry");
                    setPage("key-import");
                    return;
                  }
                  if (backupSubview === "password") {
                    backupSessionToPasswordEntry(backupSession);
                  }
                  setBackupDirection("backward");
                  setReturningFromSecurity(false);
                  setPage("backup");
                },
                next: (runtimeIds) => {
                  const ids = Array.from(runtimeIds);
                  setReadyRuntimeIds(ids);
                  // Harness install can fail (Windows/PATH/network). Don't soft-lock
                  // onboarding — users can finish setup later in Settings → Agents.
                  if (ids.length === 0) {
                    complete(selectedPubkey ?? undefined);
                    return;
                  }
                  setPage("config");
                },
                navigateToAgentSettings: () => {
                  // Complete onboarding first, then delegate the Settings → Agents
                  // navigation to the parent.  The parent owns RouterProvider, so
                  // navigation from within the onboarding flow races with the
                  // router mounting — calling router.navigate() here is unsafe.
                  complete(selectedPubkey ?? undefined);
                  navigateAfterComplete?.({
                    to: "/settings",
                    search: { section: "agents" },
                  });
                },
              }}
              direction="forward"
              onReadyRuntimeIdsChange={handleReadyRuntimeIdsChange}
            />
          ) : (
            <DefaultConfigStep
              actions={{
                back: () => setPage("setup"),
                complete: () => complete(selectedPubkey ?? undefined),
              }}
              direction="forward"
              readyRuntimeIds={readyRuntimeIds}
            />
          )}
        </div>
      </OnboardingFooterProvider>
    </div>
  );
}
