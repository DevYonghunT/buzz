import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

const LOCALE_STORAGE_KEY = "buzz-ui-locale.v1";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
  await page.addInitScript(
    ({ key }) => {
      if (window.localStorage.getItem(key) === null) {
        window.localStorage.setItem(key, "en");
      }
    },
    { key: LOCALE_STORAGE_KEY },
  );
});

test("switches the interface to Korean and persists it across reloads", async ({
  page,
}) => {
  await page.goto("/");
  await openSettings(page, "appearance");

  await expect(
    page.getByTestId("settings-theme").getByRole("heading", {
      name: "Appearance",
    }),
  ).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("lang", "en");

  await page.getByTestId("interface-language-trigger").click();
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
  await expect
    .poll(() =>
      page.evaluate(
        (key) => window.localStorage.getItem(key),
        LOCALE_STORAGE_KEY,
      ),
    )
    .toBe("ko");

  await page.reload();

  await expect(page.getByTestId("settings-view")).toBeVisible();
  await expect(
    page.getByTestId("settings-theme").getByRole("heading", {
      name: "화면 및 언어",
    }),
  ).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("lang", "ko-KR");
});
