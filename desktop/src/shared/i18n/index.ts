import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import {
  FALLBACK_APP_LOCALE,
  getInitialAppLocale,
  normalizeAppLocale,
  syncDocumentLocale,
} from "@/shared/i18n/locale";
import { APP_I18N_NAMESPACES, appI18nResources } from "@/shared/i18n/resources";

void i18n.use(initReactI18next).init({
  defaultNS: APP_I18N_NAMESPACES,
  fallbackLng: FALLBACK_APP_LOCALE,
  initAsync: false,
  interpolation: {
    escapeValue: false,
  },
  lng: getInitialAppLocale(),
  ns: APP_I18N_NAMESPACES,
  nsSeparator: ".",
  resources: appI18nResources,
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
export type {
  AppI18nNamespace,
  TranslationKey,
} from "@/shared/i18n/resources";
