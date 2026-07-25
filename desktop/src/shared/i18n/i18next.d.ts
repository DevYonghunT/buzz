import type { en } from "@/shared/i18n/locales/en";

declare module "i18next" {
  interface CustomTypeOptions {
    defaultNS: readonly ["app", "settings", "appearance"];
    keySeparator: ".";
    nsSeparator: ".";
    resources: typeof en;
    returnNull: false;
    strictKeyChecks: true;
  }
}
