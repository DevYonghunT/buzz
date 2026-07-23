export const APP_LOCALES = ["ko", "en"] as const;

export type AppLocale = (typeof APP_LOCALES)[number];

export const DEFAULT_APP_LOCALE: AppLocale = "ko";
export const FALLBACK_APP_LOCALE: AppLocale = "en";
export const LOCALE_STORAGE_KEY = "buzz-ui-locale.v1";

type LocaleStorage = Pick<Storage, "getItem" | "setItem">;

export function normalizeAppLocale(value: unknown): AppLocale | null {
  if (typeof value !== "string") {
    return null;
  }

  const language = value.trim().toLowerCase().split(/[-_]/, 1)[0];
  return APP_LOCALES.find((locale) => locale === language) ?? null;
}

export function readStoredAppLocale(
  storage: Pick<LocaleStorage, "getItem"> | null | undefined,
): AppLocale | null {
  if (!storage) {
    return null;
  }

  try {
    return normalizeAppLocale(storage.getItem(LOCALE_STORAGE_KEY));
  } catch {
    return null;
  }
}

export function persistAppLocale(
  locale: AppLocale,
  storage: Pick<LocaleStorage, "setItem"> | null | undefined,
): void {
  if (!storage) {
    return;
  }

  try {
    storage.setItem(LOCALE_STORAGE_KEY, locale);
  } catch {
    // A blocked or full localStorage must not prevent changing the UI language
    // for the current session.
  }
}

export function resolveInitialAppLocale(storedLocale: unknown): AppLocale {
  return normalizeAppLocale(storedLocale) ?? DEFAULT_APP_LOCALE;
}

export function getInitialAppLocale(): AppLocale {
  if (typeof window === "undefined") {
    return DEFAULT_APP_LOCALE;
  }

  try {
    return resolveInitialAppLocale(readStoredAppLocale(window.localStorage));
  } catch {
    return DEFAULT_APP_LOCALE;
  }
}

export function syncDocumentLocale(locale: AppLocale): void {
  if (typeof document === "undefined") {
    return;
  }

  document.documentElement.lang = locale === "ko" ? "ko-KR" : "en";
  document.documentElement.dir = "ltr";
}
