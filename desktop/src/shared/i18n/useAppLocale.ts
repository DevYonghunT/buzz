import { useCallback } from "react";
import { useTranslation } from "react-i18next";

import { i18n } from "@/shared/i18n";
import {
  type AppLocale,
  FALLBACK_APP_LOCALE,
  getLocaleStorageFromHost,
  normalizeAppLocale,
  persistAppLocale,
} from "@/shared/i18n/locale";

export async function setAppLocale(locale: AppLocale): Promise<void> {
  persistAppLocale(
    locale,
    getLocaleStorageFromHost(typeof window === "undefined" ? null : window),
  );
  await i18n.changeLanguage(locale);
}

export function useAppLocale(): {
  locale: AppLocale;
  setLocale: (locale: AppLocale) => Promise<void>;
} {
  const { i18n: currentI18n } = useTranslation();
  const locale =
    normalizeAppLocale(currentI18n.resolvedLanguage ?? currentI18n.language) ??
    FALLBACK_APP_LOCALE;
  const setLocale = useCallback(
    (nextLocale: AppLocale) => setAppLocale(nextLocale),
    [],
  );

  return { locale, setLocale };
}
