/**
 * Shared date/time formatters for the message timeline.
 *
 * - `formatTime` — short clock time ("2:34 PM"), used in message rows. The
 *   Korean day-period marker is whatever the running ICU's CLDR data says, so
 *   do not assume "오전"/"오후" — current releases render "AM"/"PM" there too.
 * - `formatTimeWithoutDayPeriod` — the same clock time with that marker dropped,
 *   wherever in the string the locale puts it.
 * - `formatFullDateTime` — verbose string for tooltips.
 * - `formatDayHeading` — label for day dividers / sticky headers.
 *   Returns "Today", "Yesterday", or a date.
 * - `isSameDay` — compare two unix-second timestamps.
 *
 * Every function takes the app locale explicitly. These used to hold
 * module-level `Intl.DateTimeFormat("en-US")` constants, which froze the
 * timeline — the most-read text in the app — to English *and* resolved once at
 * import, so even a correct locale there could not have survived the user
 * switching language. Taking the locale per call keeps them pure functions of
 * their arguments; `shared/i18n/formatters` caches the underlying `Intl`
 * instances by locale and options, so there is no per-render construction cost.
 *
 * English output is preserved exactly as it shipped: ordinal dates
 * ("May 19th"), "3 hours ago", "on May 19th". Korean has no ordinal suffix and
 * builds dates in year→month→day order, so it uses the `Intl` forms instead.
 * Adding Korean must not restyle English product copy, which is why the ordinal
 * path below is English-only rather than replaced by `Intl` for both.
 */

import { i18n } from "@/shared/i18n";
import {
  formatDateTime,
  formatRelativeTime,
  getDateTimeFormatter,
} from "@/shared/i18n/formatters";
import type { AppLocale } from "@/shared/i18n/locale";

const TIME_OPTIONS: Intl.DateTimeFormatOptions = {
  hour: "numeric",
  minute: "2-digit",
};

const FULL_DATE_TIME_OPTIONS: Intl.DateTimeFormatOptions = {
  weekday: "long",
  year: "numeric",
  month: "long",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
};

// Both bind `t` to the locale we were handed rather than i18next's ambient
// language, so these stay pure functions of their arguments.
//
// Split by key rather than taking a `TranslationKey` union: `strictKeyChecks`
// types the options argument per key, so a union key has no single matching
// overload. Literal keys also mean a typo or a key removed from the catalog is
// a compile error here.
function label(
  locale: AppLocale,
  key: "time.today" | "time.yesterday" | "time.justNow",
): string {
  return i18n.getFixedT(locale)(key);
}

function labelWithDate(locale: AppLocale, date: string): string {
  return i18n.getFixedT(locale)("time.onDate", { date });
}

/** Short clock time, e.g. "2:34 PM". */
export function formatTime(unixSeconds: number, locale: AppLocale): string {
  return formatDateTime(new Date(unixSeconds * 1_000), locale, TIME_OPTIONS);
}

const dayPeriodMarkerCache = new Map<AppLocale, readonly string[]>();

/**
 * The day-period markers this locale actually uses, asked of ICU rather than
 * hardcoded. Korean has been "오전"/"오후" and, in current CLDR, "AM"/"PM"; the
 * marker is data that changes between ICU releases and differs between the
 * webview and Node, so reading it back is the only stable way to strip it.
 */
function dayPeriodMarkers(locale: AppLocale): readonly string[] {
  const cached = dayPeriodMarkerCache.get(locale);
  if (cached) {
    return cached;
  }

  const formatter = getDateTimeFormatter(locale, TIME_OPTIONS);
  const markers = [0, 12]
    .map(
      (hour) =>
        formatter
          .formatToParts(new Date(2026, 0, 1, hour))
          .find((part) => part.type === "dayPeriod")?.value,
    )
    .filter((marker): marker is string => Boolean(marker));

  dayPeriodMarkerCache.set(locale, markers);
  return markers;
}

/**
 * Short clock time with the day-period marker removed, e.g. "2:34".
 *
 * The old implementation was `/[\s  ]*(?:AM|PM)$/` — anchored to the
 * end, so it only ever matched English. Korean puts the marker in FRONT of the
 * clock time, so the regex silently left it in place and `hideDayPeriod` did
 * nothing there. Removing the marker wherever it sits fixes both.
 */
export function formatTimeWithoutDayPeriod(
  time: string,
  locale: AppLocale,
): string {
  return dayPeriodMarkers(locale)
    .reduce((remaining, marker) => remaining.split(marker).join(""), time)
    .replace(/[\s  ]+/g, " ")
    .trim();
}

/** Full date + time for tooltips. */
export function formatFullDateTime(
  unixSeconds: number,
  locale: AppLocale,
): string {
  return formatDateTime(
    new Date(unixSeconds * 1_000),
    locale,
    FULL_DATE_TIME_OPTIONS,
  );
}

/**
 * Human-friendly day label for dividers and sticky headers.
 * Returns "Today", "Yesterday", a current-year date, or a date carrying its year.
 */
export function formatDayHeading(
  unixSeconds: number,
  locale: AppLocale,
): string {
  const date = new Date(unixSeconds * 1_000);
  const now = new Date();

  if (isSameDayDate(date, now)) {
    return label(locale, "time.today");
  }

  const yesterday = new Date(now);
  yesterday.setDate(yesterday.getDate() - 1);
  if (isSameDayDate(date, yesterday)) {
    return label(locale, "time.yesterday");
  }

  const includeYear = date.getFullYear() !== now.getFullYear();

  if (locale !== "en") {
    return formatDateTime(date, locale, {
      weekday: "long",
      month: "long",
      day: "numeric",
      ...(includeYear ? { year: "numeric" } : {}),
    });
  }

  const weekday = formatDateTime(date, locale, { weekday: "long" });
  const dateLabel = `${weekday}, ${formatMonthDayOrdinal(date, locale, "long")}`;
  return includeYear ? `${dateLabel}, ${date.getFullYear()}` : dateLabel;
}

/** True when two unix-second timestamps fall on the same calendar day (local time). */
export function isSameDay(a: number, b: number): boolean {
  return isSameDayDate(new Date(a * 1_000), new Date(b * 1_000));
}

/**
 * Unix-seconds timestamp of local midnight for the calendar day containing
 * `unixSeconds`. Two timestamps on the same calendar day map to the same value,
 * so it is a stable identifier for a day group that does not shift when an
 * older message is prepended into that day.
 */
export function startOfLocalDaySeconds(unixSeconds: number): number {
  const date = new Date(unixSeconds * 1_000);
  date.setHours(0, 0, 0, 0);
  return Math.floor(date.getTime() / 1_000);
}

/** Short month + day, e.g. "May 19th" or "5월 19일". */
export function formatShortMonthDayOrdinal(
  unixSeconds: number,
  locale: AppLocale,
): string {
  return formatMonthDayOrdinal(new Date(unixSeconds * 1_000), locale, "short");
}

/**
 * Relative thread-summary timestamp with expanded units, e.g. "3 hours ago" or
 * "3시간 전", falling back to a date for older replies.
 */
export function formatThreadSummaryLastReplyTime(
  unixSeconds: number,
  locale: AppLocale,
  nowSeconds = Date.now() / 1_000,
): string {
  const diff = Math.max(0, nowSeconds - unixSeconds);

  if (diff < 60) return label(locale, "time.justNow");
  if (diff < 3_600) return formatAgo(Math.floor(diff / 60), "minute", locale);
  if (diff < 86_400) return formatAgo(Math.floor(diff / 3_600), "hour", locale);
  if (diff < 604_800)
    return formatAgo(Math.floor(diff / 86_400), "day", locale);

  return labelWithDate(locale, formatShortMonthDayOrdinal(unixSeconds, locale));
}

function isSameDayDate(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

function formatMonthDayOrdinal(
  date: Date,
  locale: AppLocale,
  monthWidth: "long" | "short",
): string {
  if (locale !== "en") {
    // Always "long" for Korean: both widths render "5월", while "numeric" would
    // produce the bare "5. 19." pattern, which reads as a serial number rather
    // than a date next to timeline copy.
    return formatDateTime(date, locale, { month: "long", day: "numeric" });
  }

  const month = formatDateTime(date, locale, { month: monthWidth });
  return `${month} ${date.getDate()}${ordinalSuffix(date.getDate())}`;
}

function formatAgo(
  value: number,
  unit: Intl.RelativeTimeFormatUnit,
  locale: AppLocale,
): string {
  // `Intl.RelativeTimeFormat` supplies the plural rules and the word order,
  // which the previous `${value} ${unit}${value === 1 ? "" : "s"} ago` could
  // only ever get right for English. It renders the same English strings.
  return formatRelativeTime(-value, unit, locale, { numeric: "always" });
}

function ordinalSuffix(day: number): string {
  const lastTwoDigits = day % 100;
  if (lastTwoDigits >= 11 && lastTwoDigits <= 13) {
    return "th";
  }

  switch (day % 10) {
    case 1:
      return "st";
    case 2:
      return "nd";
    case 3:
      return "rd";
    default:
      return "th";
  }
}
