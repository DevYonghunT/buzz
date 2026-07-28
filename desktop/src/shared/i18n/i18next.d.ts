import type { en } from "@/shared/i18n/locales/en";
import type { AppI18nNamespaces } from "@/shared/i18n/resources";

declare module "i18next" {
  interface CustomTypeOptions {
    defaultNS: AppI18nNamespaces;
    keySeparator: ".";
    nsSeparator: ".";
    resources: typeof en;
    returnNull: false;
    strictKeyChecks: true;
  }
}
