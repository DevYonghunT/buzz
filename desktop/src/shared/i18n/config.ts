import type { InitOptions } from "i18next";

import {
  APP_LOCALES,
  type AppLocale,
  FALLBACK_APP_LOCALE,
} from "@/shared/i18n/locale";
import { APP_I18N_NAMESPACES, appI18nResources } from "@/shared/i18n/resources";

/**
 * The options `shared/i18n` initializes i18next with.
 *
 * Kept in its own side-effect-free module so tests can assert against the
 * *shipped* configuration. Importing `shared/i18n` runs `i18n.init()` at module
 * load and reads `window`, so a test that imported it would be initializing the
 * app's singleton rather than inspecting its settings — which is why the
 * catalog tests used to re-declare their own options and, in doing so, silently
 * dropped `fallbackLng`. Nothing then covered the Korean→English fallback the
 * product depends on.
 *
 * `lng` is a parameter rather than a baked-in value because resolving it reads
 * `localStorage` and `navigator`; the caller decides where the initial locale
 * comes from.
 */
export function createAppI18nInitOptions(lng: AppLocale): InitOptions {
  return {
    defaultNS: APP_I18N_NAMESPACES,
    fallbackLng: FALLBACK_APP_LOCALE,
    initAsync: false,
    interpolation: {
      escapeValue: false,
    },
    lng,
    ns: APP_I18N_NAMESPACES,
    nsSeparator: ".",
    resources: appI18nResources,
    returnNull: false,
    supportedLngs: [...APP_LOCALES],
  };
}
