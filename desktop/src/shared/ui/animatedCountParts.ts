import { formatNumber } from "@/shared/i18n/formatters";
import type { AppLocale } from "@/shared/i18n/locale";

export type AnimatedCountSlot = {
  current: string;
  isDigit: boolean;
  place: number;
  previous: string;
};

export function normalizeAnimatedCount(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.trunc(value));
}

export function formatAnimatedCount(
  value: number,
  locale: AppLocale = "en",
): string {
  return formatNumber(normalizeAnimatedCount(value), locale, {
    maximumFractionDigits: 0,
  });
}

export function isAsciiDigit(value: string): boolean {
  return value.length === 1 && value >= "0" && value <= "9";
}

export function getAnimatedCountSlots(
  previous: string,
  current: string,
): AnimatedCountSlot[] {
  const width = Math.max(previous.length, current.length);
  const previousOffset = width - previous.length;
  const currentOffset = width - current.length;

  return Array.from({ length: width }, (_, index) => {
    const previousCharacter =
      index >= previousOffset ? previous[index - previousOffset] : "";
    const currentCharacter =
      index >= currentOffset ? current[index - currentOffset] : "";

    return {
      current: currentCharacter ?? "",
      isDigit:
        isAsciiDigit(previousCharacter) || isAsciiDigit(currentCharacter),
      place: width - index - 1,
      previous: previousCharacter ?? "",
    };
  });
}
