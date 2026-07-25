import { en } from "@/shared/i18n/locales/en";
import { ko } from "@/shared/i18n/locales/ko";
import type { TranslationLeafKey } from "@/shared/i18n/types";

export const APP_I18N_NAMESPACES = [
  "app",
  "settings",
  "appearance",
] as const satisfies readonly (keyof typeof en)[];

export type AppI18nNamespace = (typeof APP_I18N_NAMESPACES)[number];

export type TranslationKey = {
  [Namespace in AppI18nNamespace]: `${Namespace}.${TranslationLeafKey<
    (typeof en)[Namespace]
  >}`;
}[AppI18nNamespace];

export const appI18nResources = {
  en,
  ko,
} as const;
