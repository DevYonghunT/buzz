import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import {
  FALLBACK_APP_LOCALE,
  getInitialAppLocale,
  normalizeAppLocale,
  syncDocumentLocale,
} from "@/shared/i18n/locale";
import { en } from "@/shared/i18n/locales/en";
import { ko } from "@/shared/i18n/locales/ko";

void i18n.use(initReactI18next).init({
  fallbackLng: FALLBACK_APP_LOCALE,
  initAsync: false,
  interpolation: {
    escapeValue: false,
  },
  lng: getInitialAppLocale(),
  resources: {
    en: { translation: en },
    ko: { translation: ko },
  },
  returnNull: false,
  supportedLngs: ["ko", "en"],
});

function updateDocumentLocale(language: string): void {
  const locale = normalizeAppLocale(language) ?? FALLBACK_APP_LOCALE;
  syncDocumentLocale(locale);
}

updateDocumentLocale(i18n.resolvedLanguage ?? i18n.language);
i18n.on("languageChanged", updateDocumentLocale);

export { i18n };
