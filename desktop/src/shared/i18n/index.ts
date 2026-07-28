import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import { createAppI18nInitOptions } from "@/shared/i18n/config";
import {
  FALLBACK_APP_LOCALE,
  getInitialAppLocale,
  normalizeAppLocale,
  syncDocumentLocale,
} from "@/shared/i18n/locale";

void i18n
  .use(initReactI18next)
  .init(createAppI18nInitOptions(getInitialAppLocale()));

function updateDocumentLocale(language: string): void {
  const locale = normalizeAppLocale(language) ?? FALLBACK_APP_LOCALE;
  syncDocumentLocale(locale);
}

updateDocumentLocale(i18n.resolvedLanguage ?? i18n.language);
i18n.on("languageChanged", updateDocumentLocale);

// The catalogs are compiled into the bundle, so a spec cannot ship a holed one.
// Handing the live instance to E2E lets a spec punch the hole at runtime and
// check what the user actually sees. Gated on `__BUZZ_E2E__`, which the bridge
// sets from an init script before any module runs — same shape as
// `__BUZZ_E2E_QUERY_CLIENT__` in `app/App.tsx`.
if (
  typeof window !== "undefined" &&
  (window as Window & { __BUZZ_E2E__?: unknown }).__BUZZ_E2E__
) {
  (window as Window & { __BUZZ_E2E_I18N__?: typeof i18n }).__BUZZ_E2E_I18N__ =
    i18n;
}

export { i18n };
export type {
  AppI18nNamespace,
  TranslationKey,
} from "@/shared/i18n/resources";
