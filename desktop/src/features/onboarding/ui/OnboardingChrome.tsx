import { useTranslation } from "react-i18next";

import { SchoolXOnboardingBrand } from "./SchoolXOnboardingBrand";

/**
 * Positions in the first-launch flow: landing, identity/key, harness setup,
 * default config, community choice, community profile, meet the team. Password
 * backup is an optional subview of identity/key, not another position.
 */
export const TOTAL_ONBOARDING_PAGES = 7;

/** Shared pill shape for every onboarding primary CTA. */
const ONBOARDING_CTA_SHAPE = "h-[2.375rem] rounded-full px-6";

/**
 * Primary-CTA styling for onboarding pages, using the active SchoolX theme.
 */
export const ONBOARDING_PRIMARY_CTA_CLASS = `${ONBOARDING_CTA_SHAPE} bg-primary text-primary-foreground hover:bg-primary/90 hover:text-primary-foreground`;

/** Inverted primary action used only on dark backup-security surfaces. */
export const ONBOARDING_SECURITY_PRIMARY_CTA_CLASS = `${ONBOARDING_CTA_SHAPE} bg-white text-black/80 hover:bg-white/90 hover:text-black`;

/**
 * Primary action for the landing screen. Kept as a separate export so the
 * landing layout can evolve without changing the in-step action contract.
 */
export const ONBOARDING_LANDING_CTA_CLASS = ONBOARDING_PRIMARY_CTA_CLASS;

/** Shared quiet pill for secondary actions throughout onboarding. */
export const ONBOARDING_SECONDARY_CTA_CLASS =
  "h-9 rounded-full bg-foreground/10 px-6 text-foreground hover:bg-foreground/15 hover:text-foreground";

/**
 * Icon-control styling for onboarding surfaces that sit on a textured card.
 */
export const ONBOARDING_INK_ICON_CLASS =
  "text-muted-foreground hover:bg-transparent hover:text-foreground";

/** Icon controls on the dark noisy backup surfaces stay visually unboxed. */
export const ONBOARDING_SECURITY_ICON_CLASS =
  "text-muted-foreground hover:bg-transparent hover:text-foreground";

/**
 * Shared SchoolX chrome shown on every page after the landing screen. The
 * active page reads as a longer bar; inactive pages are dots.
 */
export function OnboardingChrome({
  current,
  total = TOTAL_ONBOARDING_PAGES,
}: {
  current: number;
  total?: number;
}) {
  const { t } = useTranslation();

  return (
    <div
      aria-hidden
      className="pointer-events-none fixed inset-x-0 top-12 z-10 flex items-center px-6 text-foreground"
    >
      <SchoolXOnboardingBrand
        className="shrink-0"
        productName={t("app.productName")}
      />
      <div
        className="absolute left-1/2 top-1/2 flex -translate-x-1/2 -translate-y-1/2 items-center gap-2"
        data-testid="onboarding-step-dots"
      >
        {Array.from({ length: total }, (_, i) => i + 1).map((position) => (
          <span
            className={
              position === current
                ? "block h-1.5 w-7 rounded-full bg-foreground"
                : "block h-1.5 w-1.5 rounded-full bg-foreground/30"
            }
            key={position}
          />
        ))}
      </div>
    </div>
  );
}
