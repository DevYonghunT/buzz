export const APP_LOCALES = ["ko", "en"] as const;

export type AppLocale = (typeof APP_LOCALES)[number];

export const DEFAULT_APP_LOCALE: AppLocale = "ko";
export const FALLBACK_APP_LOCALE: AppLocale = "en";
export const LOCALE_STORAGE_KEY = "buzz-ui-locale.v1";

export const APP_LOCALE_FORMAT_TAGS: Record<AppLocale, string> = {
  en: "en-US",
  ko: "ko-KR",
};

const APP_LOCALE_DOCUMENT_TAGS: Record<AppLocale, string> = {
  en: "en",
  ko: "ko-KR",
};

export type LocaleStorage = Pick<Storage, "getItem" | "setItem">;

type LocaleStorageHost = {
  readonly localStorage: LocaleStorage;
};

type LocaleNavigator = {
  readonly language?: string;
  readonly languages?: readonly string[];
};

export function normalizeAppLocale(value: unknown): AppLocale | null {
  if (typeof value !== "string") {
    return null;
  }

  const candidate = value.trim().replaceAll("_", "-");
  if (!candidate) {
    return null;
  }

  const exactLocale = candidate.toLowerCase();
  if (exactLocale === "ko" || exactLocale === "en") {
    return exactLocale;
  }

  try {
    const [canonicalLocale] = Intl.getCanonicalLocales(candidate);
    if (!canonicalLocale) {
      return null;
    }

    const language = new Intl.Locale(canonicalLocale).language.toLowerCase();
    return language === "ko" || language === "en" ? language : null;
  } catch {
    return null;
  }
}

export function getLocaleStorageFromHost(
  host: LocaleStorageHost | null | undefined,
): LocaleStorage | null {
  if (!host) {
    return null;
  }

  try {
    return host.localStorage;
  } catch {
    return null;
  }
}

export function readPreferredSystemLocales(
  localeNavigator: LocaleNavigator | null | undefined,
): string[] {
  if (!localeNavigator) {
    return [];
  }

  const preferredLocales: string[] = [];
  try {
    preferredLocales.push(...(localeNavigator.languages ?? []));
  } catch {
    // Some embedded or privacy-restricted webviews block navigator metadata.
  }

  try {
    const primaryLocale = localeNavigator.language;
    if (primaryLocale && !preferredLocales.includes(primaryLocale)) {
      preferredLocales.push(primaryLocale);
    }
  } catch {
    // Fall through to the product default when no system locale is readable.
  }

  return preferredLocales;
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

export function resolveInitialAppLocale(
  storedLocale: unknown,
  preferredSystemLocales: readonly unknown[] = [],
): AppLocale {
  const normalizedStoredLocale = normalizeAppLocale(storedLocale);
  if (normalizedStoredLocale) {
    return normalizedStoredLocale;
  }

  for (const preferredLocale of preferredSystemLocales) {
    const normalizedSystemLocale = normalizeAppLocale(preferredLocale);
    if (normalizedSystemLocale) {
      return normalizedSystemLocale;
    }
  }

  return DEFAULT_APP_LOCALE;
}

export function getInitialAppLocale(): AppLocale {
  if (typeof window === "undefined") {
    return DEFAULT_APP_LOCALE;
  }

  const storage = getLocaleStorageFromHost(window);
  const preferredSystemLocales =
    typeof navigator === "undefined"
      ? []
      : readPreferredSystemLocales(navigator);

  return resolveInitialAppLocale(
    readStoredAppLocale(storage),
    preferredSystemLocales,
  );
}

export function syncDocumentLocale(locale: AppLocale): void {
  if (typeof document === "undefined") {
    return;
  }

  document.documentElement.lang = APP_LOCALE_DOCUMENT_TAGS[locale];
  document.documentElement.dir = "ltr";
}
