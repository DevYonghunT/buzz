import type { ComponentPropsWithoutRef } from "react";

/** Canonical local WebView asset generated from `brand/schoolx-mark.svg`. */
export const SCHOOLX_MARK_ASSET_PATH = "/brand/schoolx-mark.svg";

/** Props for the local SchoolX product mark image. */
export type SchoolXMarkProps = Omit<
  ComponentPropsWithoutRef<"img">,
  "alt" | "aria-hidden" | "aria-label" | "src"
> & {
  /** Remove the mark from the accessibility tree when nearby text names it. */
  decorative?: boolean;
  /** Localized accessible name for a meaningful mark. */
  ariaLabel?: string;
};

/**
 * Render the canonical SchoolX product mark without embedding or duplicating
 * its SVG source. Decorative marks use an empty alternative; meaningful marks
 * use the provided localized label or the product-name fallback.
 */
export function SchoolXMark({
  ariaLabel = "SchoolX",
  decorative = false,
  ...imageProps
}: SchoolXMarkProps) {
  return (
    <img
      {...imageProps}
      src={SCHOOLX_MARK_ASSET_PATH}
      alt={decorative ? "" : ariaLabel}
      aria-hidden={decorative ? true : undefined}
      data-testid="schoolx-mark"
    />
  );
}
