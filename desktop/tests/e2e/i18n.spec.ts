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
