import { en } from "@/shared/i18n/locales/en";
import { ko } from "@/shared/i18n/locales/ko";
import type { TranslationLeafKey } from "@/shared/i18n/types";

// Adding a namespace here means adding it to BOTH catalogs. A namespace present
// in `en` and missing from `ko` is not rescued by `fallbackLng` — every key in
// it renders as a raw key path for Korean users. See the characterization test
// in `resources.test.mjs`.
export const APP_I18N_NAMESPACES = [
  "app",
  "settings",
  "time",
  "appearance",
] as const satisfies readonly (keyof typeof en)[];

export type AppI18nNamespace = (typeof APP_I18N_NAMESPACES)[number];

/**
 * The namespace tuple itself, so `i18next.d.ts` can declare `defaultNS` from
 * this list instead of restating it. The two used to be separate literals, and
 * adding a namespace to only one of them made every key in it fail to typecheck
 * at the call sites while the runtime happily resolved it.
 */
export type AppI18nNamespaces = typeof APP_I18N_NAMESPACES;

export type TranslationKey = {
  [Namespace in AppI18nNamespace]: `${Namespace}.${TranslationLeafKey<
    (typeof en)[Namespace]
  >}`;
}[AppI18nNamespace];

export const appI18nResources = {
  en,
  ko,
} as const;
