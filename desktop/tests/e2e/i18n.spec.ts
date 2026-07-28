import { expect, test, type Page } from "@playwright/test";

import { LOCALE_STORAGE_KEY } from "../../src/shared/i18n/locale";
import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

async function expectStoredLocale(page: Page, expectedLocale: "en" | "ko") {
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        LOCALE_STORAGE_KEY,
      ),
    )
    .toBe(expectedLocale);
}

/**
 * Delete one key from the live Korean catalog, the way a translator forgetting a
 * string would leave it. The catalogs are compiled into the bundle, so the hole
 * has to be punched at runtime through the E2E handle `shared/i18n` exposes.
 *
 * Remove-then-add, not `addResourceBundle(..., overwrite)`: overwrite replaces
 * the keys it is handed and leaves the rest of the bundle in place, so passing
 * an object with the key omitted deletes nothing.
 */
async function dropKoreanTranslationKey(
  page: Page,
  namespace: string,
  key: string,
) {
  await page.evaluate(
    ([ns, leaf]) => {
      const i18n = (
        window as Window & {
          __BUZZ_E2E_I18N__?: {
            addResourceBundle: (
              lng: string,
              ns: string,
              bundle: Record<string, unknown>,
            ) => void;
            emit: (event: string, language: string) => void;
            getResourceBundle: (
              lng: string,
              ns: string,
            ) => Record<string, unknown>;
            language: string;
            removeResourceBundle: (lng: string, ns: string) => void;
            resolvedLanguage?: string;
          };
        }
      ).__BUZZ_E2E_I18N__;
      if (!i18n) {
        throw new Error("__BUZZ_E2E_I18N__ is not exposed");
      }

      const bundle = { ...i18n.getResourceBundle("ko", ns) };
      delete bundle[leaf];
      i18n.removeResourceBundle("ko", ns);
      i18n.addResourceBundle("ko", ns, bundle);
      // Mounted components hold rendered strings; nudge react-i18next to re-run
      // `t()` now that the catalog underneath it changed. `bindI18nStore` is off,
      // so the bundle swap alone does not re-render.
      //
      // Emit with the *current* language, not bare. `shared/i18n` also listens
      // for `languageChanged` to set `<html lang>`, and an argument-less emit
      // makes it normalize `undefined` and fall back to `en` — which would flip
      // the attribute this spec is asserting on.
      i18n.emit("languageChanged", i18n.resolvedLanguage ?? i18n.language);
    },
    [namespace, key] as const,
  );
}

test.describe("missing Korean translations", () => {
  test("render the English string rather than the raw key path", async ({
    page,
  }) => {
    await installMockBridge(page, undefined, { appLocale: "ko" });
    await page.goto("/");
    await openSettings(page, "appearance");

    const heading = page
      .getByTestId("settings-theme")
      .getByRole("heading", { name: "화면 및 언어" });
    await expect(heading).toBeVisible();

    await dropKoreanTranslationKey(page, "appearance", "title");

    // English, and specifically not "appearance.title" on screen.
    await expect(
      page
        .getByTestId("settings-theme")
        .getByRole("heading", { name: "Appearance" }),
    ).toBeVisible();
    await expect(page.getByTestId("settings-theme")).not.toContainText(
      "appearance.title",
    );

    // The rest of the screen stays Korean — one gap must not flip the UI to
    // English wholesale.
    await expect(page.getByTestId("settings-back-to-app")).toContainText(
      "앱으로 돌아가기",
    );
    await expect(page.locator("html")).toHaveAttribute("lang", "ko-KR");
  });
});

test.describe("fresh-install locale resolution", () => {
  test.describe("with a supported English OS locale", () => {
    test.use({ locale: "en-US" });

    test("uses English", async ({ page }) => {
      await installMockBridge(page, undefined, { appLocale: null });
      await page.goto("/");
      await openSettings(page, "appearance");

      await expect(page.locator("html")).toHaveAttribute("lang", "en");
      await expect(
        page.getByTestId("settings-theme").getByRole("heading", {
          name: "Appearance",
        }),
      ).toBeVisible();

      const languageTrigger = page.getByTestId("interface-language-trigger");
      await expect(languageTrigger).toHaveAccessibleName(
        "Interface language English",
      );
      await expect(languageTrigger.locator("[lang='en']")).toHaveText(
        "English",
      );

      await languageTrigger.click();
      await expect(
        page.getByTestId("interface-language-en").locator("[lang='en']"),
      ).toHaveText("English");
      await expect(
        page.getByTestId("interface-language-ko").locator("[lang='ko-KR']"),
      ).toHaveText("한국어");
    });
  });

  test.describe("with an unsupported Japanese OS locale", () => {
    test.use({ locale: "ja-JP" });

    test("uses the Korean product default", async ({ page }) => {
      await installMockBridge(page, undefined, { appLocale: null });
      await page.goto("/");
      await openSettings(page, "appearance");

      await expect(page.locator("html")).toHaveAttribute("lang", "ko-KR");
      await expect(
        page.getByTestId("settings-theme").getByRole("heading", {
          name: "화면 및 언어",
        }),
      ).toBeVisible();
      await expect(
        page.getByTestId("interface-language-trigger"),
      ).toHaveAccessibleName("인터페이스 언어 한국어");
    });
  });
});

test("switches the interface to Korean and persists it across reloads", async ({
  page,
}) => {
  await installMockBridge(page, undefined, { appLocale: "en" });
  await page.goto("/");
  await openSettings(page, "appearance");

  await expect(
    page.getByTestId("settings-theme").getByRole("heading", {
      name: "Appearance",
    }),
  ).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("lang", "en");

  const languageTrigger = page.getByTestId("interface-language-trigger");
  await expect(languageTrigger).toHaveAccessibleName(
    "Interface language English",
  );
  await languageTrigger.click();
  await page.getByTestId("interface-language-ko").click();

  await expect(
    page.getByTestId("settings-theme").getByRole("heading", {
      name: "화면 및 언어",
    }),
  ).toBeVisible();
  await expect(page.getByTestId("settings-back-to-app")).toContainText(
    "앱으로 돌아가기",
  );
  await expect(page.locator("html")).toHaveAttribute("lang", "ko-KR");
  await expect(languageTrigger).toHaveAccessibleName("인터페이스 언어 한국어");
  await expectStoredLocale(page, "ko");

  await page.reload();

  await expect(page.getByTestId("settings-view")).toBeVisible();
  await expect(
    page.getByTestId("settings-theme").getByRole("heading", {
      name: "화면 및 언어",
    }),
  ).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("lang", "ko-KR");
  await expect(
    page.getByTestId("interface-language-trigger"),
  ).toHaveAccessibleName("인터페이스 언어 한국어");
  await expectStoredLocale(page, "ko");
});

test("switches back to English with the keyboard and persists it", async ({
  page,
}) => {
  await installMockBridge(page, undefined, { appLocale: "ko" });
  await page.goto("/");
  await openSettings(page, "appearance");

  const languageTrigger = page.getByTestId("interface-language-trigger");
  await expect(languageTrigger).toHaveAccessibleName("인터페이스 언어 한국어");
  await languageTrigger.focus();
  await page.keyboard.press("Enter");

  const koreanOption = page.getByTestId("interface-language-ko");
  const englishOption = page.getByTestId("interface-language-en");
  await expect(koreanOption).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(englishOption).toBeFocused();
  await page.keyboard.press("Enter");

  await expect(
    page.getByTestId("settings-theme").getByRole("heading", {
      name: "Appearance",
    }),
  ).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
  await expect(languageTrigger).toHaveAccessibleName(
    "Interface language English",
  );
  await expectStoredLocale(page, "en");

  await page.reload();

  await expect(page.getByTestId("settings-view")).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
  await expect(
    page.getByTestId("interface-language-trigger"),
  ).toHaveAccessibleName("Interface language English");
  await expectStoredLocale(page, "en");
});
