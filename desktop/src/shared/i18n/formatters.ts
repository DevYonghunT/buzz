import { APP_LOCALE_FORMAT_TAGS, type AppLocale } from "@/shared/i18n/locale";

const dateTimeFormatterCache = new Map<string, Intl.DateTimeFormat>();
const numberFormatterCache = new Map<string, Intl.NumberFormat>();
const relativeTimeFormatterCache = new Map<string, Intl.RelativeTimeFormat>();

function formatterCacheKey(
  locale: AppLocale,
  options:
    | Intl.DateTimeFormatOptions
    | Intl.NumberFormatOptions
    | Intl.RelativeTimeFormatOptions,
): string {
  const sortedOptions = Object.entries(options).sort(([left], [right]) =>
    left.localeCompare(right),
  );
  return JSON.stringify([locale, sortedOptions]);
}

export function getDateTimeFormatter(
  locale: AppLocale,
  options: Intl.DateTimeFormatOptions = {},
): Intl.DateTimeFormat {
  const cacheKey = formatterCacheKey(locale, options);
  const cachedFormatter = dateTimeFormatterCache.get(cacheKey);
  if (cachedFormatter) {
    return cachedFormatter;
  }

  const formatter = new Intl.DateTimeFormat(
    APP_LOCALE_FORMAT_TAGS[locale],
    options,
  );
  dateTimeFormatterCache.set(cacheKey, formatter);
  return formatter;
}

export function formatDateTime(
  value: Date | number,
  locale: AppLocale,
  options: Intl.DateTimeFormatOptions = {},
): string {
  return getDateTimeFormatter(locale, options).format(value);
}

export function getNumberFormatter(
  locale: AppLocale,
  options: Intl.NumberFormatOptions = {},
): Intl.NumberFormat {
  const cacheKey = formatterCacheKey(locale, options);
  const cachedFormatter = numberFormatterCache.get(cacheKey);
  if (cachedFormatter) {
    return cachedFormatter;
  }

  const formatter = new Intl.NumberFormat(
    APP_LOCALE_FORMAT_TAGS[locale],
    options,
  );
  numberFormatterCache.set(cacheKey, formatter);
  return formatter;
}

export function formatNumber(
  value: number | bigint,
  locale: AppLocale,
  options: Intl.NumberFormatOptions = {},
): string {
  return getNumberFormatter(locale, options).format(value);
}

export function getRelativeTimeFormatter(
  locale: AppLocale,
  options: Intl.RelativeTimeFormatOptions = {},
): Intl.RelativeTimeFormat {
  const cacheKey = formatterCacheKey(locale, options);
  const cachedFormatter = relativeTimeFormatterCache.get(cacheKey);
  if (cachedFormatter) {
    return cachedFormatter;
  }

  const formatter = new Intl.RelativeTimeFormat(
    APP_LOCALE_FORMAT_TAGS[locale],
    options,
  );
  relativeTimeFormatterCache.set(cacheKey, formatter);
  return formatter;
}

export function formatRelativeTime(
  value: number,
  unit: Intl.RelativeTimeFormatUnit,
  locale: AppLocale,
  options: Intl.RelativeTimeFormatOptions = {},
): string {
  return getRelativeTimeFormatter(locale, options).format(value, unit);
}
