import { cn } from "@/shared/lib/cn";
import { SchoolXMark } from "@/shared/ui/schoolx-brand/SchoolXMark";

type SchoolXOnboardingBrandProps = {
  className?: string;
  productName: string;
  variant?: "compact" | "hero";
};

/** Shared SchoolX lockup for first-run and community-onboarding surfaces. */
export function SchoolXOnboardingBrand({
  className,
  productName,
  variant = "compact",
}: SchoolXOnboardingBrandProps) {
  const isHero = variant === "hero";
  const Element = isHero ? "h1" : "div";

  return (
    <Element
      className={cn(
        "inline-flex items-center text-balance text-foreground",
        isHero
          ? "flex-col gap-5 text-5xl font-semibold"
          : "gap-3 text-lg font-medium",
        className,
      )}
      data-testid={`schoolx-onboarding-brand-${variant}`}
    >
      <SchoolXMark className={isHero ? "size-28" : "size-9"} decorative />
      <span>{productName}</span>
    </Element>
  );
}
