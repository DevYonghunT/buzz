import { expect, test, type Locator, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";

import { hexToHsl } from "../../src/shared/theme/adaptive-theme";
import {
  SCHOOLX_PALETTE,
  SCHOOLX_THEME_CACHE_REVISION,
  SCHOOLX_THEME_ONLY_VAR_NAMES,
  createSchoolXTheme,
  type SchoolXThemeName,
  type ThemeCachePayload,
} from "../../src/shared/theme/schoolx-theme";
import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";
import { contrastRatio } from "../helpers/contrast";

const SHOTS = "test-results/schoolx-theme";
const THEME_KEY = "buzz-theme";
const CACHE_KEY = "buzz-theme-cache";
const ACCENT_KEY = "buzz-accent-color";
const FOLLOW_SYSTEM_KEY = "buzz-follow-system";
const PROJECT_ENTRY_SELECTOR =
  '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]';
const CODE_THREAD_ID = "thread-e2e-1";

type PreferenceSeed = {
  accent?: string | null;
  cache?: string | ThemeCachePayload | null;
  followSystem?: string | null;
  textScale?: number | null;
  theme?: string | null;
  viewMode?: "grid" | "list" | null;
};

type RootThemeState = {
  background: string;
  cacheTheme: string | null;
  foreground: string;
  isDark: boolean;
  primary: string;
  primaryForeground: string;
  sidebarBackground: string;
  storedAccent: string | null;
  storedFollowSystem: string | null;
  storedTheme: string | null;
  theme: string | null;
  translucent: boolean;
};

test.beforeAll(() => {
  mkdirSync(SHOTS, { recursive: true });
});

async function seedPreferences(page: Page, seed: PreferenceSeed) {
  await page.addInitScript(
    ({ accent, cache, followSystem, textScale, theme, viewMode }) => {
      const setOrRemove = (key: string, value: string | null | undefined) => {
        if (value === null || value === undefined) {
          window.localStorage.removeItem(key);
        } else {
          window.localStorage.setItem(key, value);
        }
      };

      setOrRemove("buzz-theme", theme);
      setOrRemove("buzz-follow-system", followSystem);
      setOrRemove("buzz-accent-color", accent);
      setOrRemove("buzz:text-scale", textScale?.toString());
      setOrRemove("buzz.projects.viewMode", viewMode);
      if (cache !== undefined) {
        setOrRemove(
          "buzz-theme-cache",
          typeof cache === "string"
            ? cache
            : cache
              ? JSON.stringify(cache)
              : null,
        );
      }
    },
    seed,
  );
}

async function blockApplicationBundle(page: Page) {
  await page.route("**/*.js", async (route) => {
    await route.abort("blockedbyclient");
  });
}

async function openHome(page: Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("home-inbox-list")).toBeVisible({
    timeout: 15_000,
  });
}

async function readRootThemeState(page: Page): Promise<RootThemeState> {
  return page.evaluate(
    ({ accentKey, cacheKey, followKey, themeKey }) => {
      const root = document.documentElement;
      const styles = getComputedStyle(root);
      let cacheTheme: string | null = null;
      try {
        const raw = window.localStorage.getItem(cacheKey);
        cacheTheme = raw
          ? ((JSON.parse(raw) as { themeName?: string }).themeName ?? null)
          : null;
      } catch {
        cacheTheme = null;
      }
      return {
        background: styles.getPropertyValue("--background").trim(),
        cacheTheme,
        foreground: styles.getPropertyValue("--foreground").trim(),
        isDark: root.classList.contains("dark"),
        primary: styles.getPropertyValue("--primary").trim(),
        primaryForeground: styles
          .getPropertyValue("--primary-foreground")
          .trim(),
        sidebarBackground: styles
          .getPropertyValue("--sidebar-background")
          .trim(),
        storedAccent: window.localStorage.getItem(accentKey),
        storedFollowSystem: window.localStorage.getItem(followKey),
        storedTheme: window.localStorage.getItem(themeKey),
        theme: root.getAttribute("data-buzz-theme"),
        translucent: root.hasAttribute("data-buzz-translucent"),
      };
    },
    {
      accentKey: ACCENT_KEY,
      cacheKey: CACHE_KEY,
      followKey: FOLLOW_SYSTEM_KEY,
      themeKey: THEME_KEY,
    },
  );
}

async function expectAppliedSchoolXTheme(
  page: Page,
  themeName: SchoolXThemeName,
  storedTheme: string | null = themeName,
) {
  const expected = createSchoolXTheme(themeName);
  await expect
    .poll(() => readRootThemeState(page))
    .toEqual(
      expect.objectContaining({
        background: expected.vars["--background"],
        cacheTheme: themeName,
        foreground: expected.vars["--foreground"],
        isDark: expected.isDark,
        primary: expected.vars["--primary"],
        primaryForeground: expected.vars["--primary-foreground"],
        sidebarBackground: expected.vars["--sidebar-background"],
        storedTheme,
        theme: themeName,
        translucent: false,
      }),
    );
}

function isOpaqueColor(color: string): boolean {
  if (color === "transparent" || color === "rgba(0, 0, 0, 0)") return false;
  const rgbaAlpha = /rgba\([^,]+,[^,]+,[^,]+,\s*([\d.]+)\)/.exec(color);
  if (rgbaAlpha) return Number.parseFloat(rgbaAlpha[1]) === 1;
  const modernAlpha = /\/\s*([\d.]+)\s*\)$/.exec(color);
  if (modernAlpha) return Number.parseFloat(modernAlpha[1]) === 1;
  return (
    color.startsWith("rgb(") ||
    color.startsWith("hsl(") ||
    color.startsWith("color(")
  );
}

async function expectFlatOpaqueSchoolXShell(page: Page) {
  const paint = await page.evaluate(() => {
    const appSurface = document.querySelector(".buzz-huddle-app-surface");
    const content = document.querySelector("[data-buzz-content-surface]");
    const gradientLayers = Array.from(
      document.querySelectorAll<HTMLElement>("[data-buzz-gradient]"),
    ).map((element) => {
      const style = getComputedStyle(element);
      return {
        backgroundImage: style.backgroundImage,
        display: style.display,
        opacity: style.opacity,
      };
    });
    return {
      appColor: appSurface ? getComputedStyle(appSurface).backgroundColor : "",
      appImage: appSurface ? getComputedStyle(appSurface).backgroundImage : "",
      contentColor: content ? getComputedStyle(content).backgroundColor : "",
      contentImage: content ? getComputedStyle(content).backgroundImage : "",
      gradientLayers,
    };
  });

  expect(isOpaqueColor(paint.appColor)).toBe(true);
  expect(paint.appImage).toBe("none");
  expect(paint.contentImage).toBe("none");
  expect(isOpaqueColor(paint.contentColor)).toBe(true);
  for (const layer of paint.gradientLayers) {
    expect(
      layer.display === "none" ||
        layer.opacity === "0" ||
        layer.backgroundImage === "none",
    ).toBe(true);
  }
}

async function openAppearance(page: Page, mode: "dark" | "light" | "system") {
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await page.getByTestId("settings-nav-appearance").click();
  const panel = page.getByTestId("settings-theme");
  await expect(panel).toBeVisible({ timeout: 10_000 });
  await page.getByTestId(`appearance-mode-${mode}`).click();
  return panel;
}

async function cssVariableColor(page: Page, name: string) {
  return page.evaluate((variableName) => {
    const probe = document.createElement("span");
    probe.style.color = `hsl(var(${variableName}))`;
    document.body.append(probe);
    const color = getComputedStyle(probe).color;
    probe.remove();
    return color;
  }, name);
}

async function expectPreviewMatchesRoot(page: Page, tile: Locator) {
  const background = await cssVariableColor(page, "--background");
  const sidebar = await cssVariableColor(page, "--sidebar-background");
  const svg = tile.locator("svg").first();
  await expect(svg.locator("rect").first()).toHaveCSS("fill", background);
  await expect(svg.locator('rect[width="57.375"]').first()).toHaveCSS(
    "fill",
    sidebar,
  );
}

async function openProjectsRepository(page: Page) {
  await page.getByTestId("open-projects-view").click();
  await page.getByRole("button", { name: "Repositories", exact: true }).click();
  const projectEntry = page.locator(PROJECT_ENTRY_SELECTOR).first();
  await expect(projectEntry).toBeVisible({ timeout: 10_000 });
  return projectEntry;
}

async function openCodeWorkspace(page: Page, keyboardOnly = false) {
  const projectEntry = await openProjectsRepository(page);
  const action = projectEntry.getByRole("button", {
    name: "Open buzz in SchoolX Code",
    exact: true,
  });
  await action.scrollIntoViewIfNeeded();
  if (keyboardOnly) {
    await action.focus();
    await expect(action).toBeFocused();
    await action.press("Enter");
  } else {
    await action.click();
  }
  await expect(page).toHaveURL(/\/#\/projects\/[^/]+\/code(?:\?|$)/);
  await expect(
    page.getByRole("navigation", { name: "Code project breadcrumb" }),
  ).toBeVisible();
}

async function expectInsideViewport(page: Page, locator: Locator) {
  await locator.scrollIntoViewIfNeeded();
  await expect(locator).toBeVisible();
  const box = await locator.boundingBox();
  const viewport = page.viewportSize();
  expect(box).not.toBeNull();
  expect(viewport).not.toBeNull();
  if (!box || !viewport) throw new Error("Missing viewport geometry");
  expect(box.x).toBeGreaterThanOrEqual(-0.5);
  expect(box.y).toBeGreaterThanOrEqual(-0.5);
  expect(box.x + box.width).toBeLessThanOrEqual(viewport.width + 0.5);
  expect(box.y + box.height).toBeLessThanOrEqual(viewport.height + 0.5);
}

async function expectNoDocumentOverflow(page: Page) {
  const geometry = await page.evaluate(() => ({
    clientHeight: document.documentElement.clientHeight,
    clientWidth: document.documentElement.clientWidth,
    scrollHeight: document.documentElement.scrollHeight,
    scrollWidth: document.documentElement.scrollWidth,
  }));
  expect(geometry.scrollWidth).toBeLessThanOrEqual(geometry.clientWidth + 1);
  expect(geometry.scrollHeight).toBeLessThanOrEqual(geometry.clientHeight + 1);
}

test("canonical SchoolX anchors retain the fixed contrast contract", () => {
  expect(
    contrastRatio(SCHOOLX_PALETTE.ink, SCHOOLX_PALETTE.parchment),
  ).toBeCloseTo(12.59, 2);
  expect(
    contrastRatio(SCHOOLX_PALETTE.parchment, SCHOOLX_PALETTE.pine),
  ).toBeCloseTo(6.98, 2);
  expect(contrastRatio("#FFFFFF", SCHOOLX_PALETTE.terracotta)).toBeCloseTo(
    4.6,
    2,
  );
  expect(
    contrastRatio(SCHOOLX_PALETTE.ink, SCHOOLX_PALETTE.terracottaDark),
  ).toBeCloseTo(4.77, 2);
  expect(
    contrastRatio(SCHOOLX_PALETTE.ink, SCHOOLX_PALETTE.warmGold),
  ).toBeCloseTo(6.76, 2);
  expect(contrastRatio(SCHOOLX_PALETTE.ink, SCHOOLX_PALETTE.sage)).toBeCloseTo(
    4.58,
    2,
  );
  expect(
    contrastRatio(SCHOOLX_PALETTE.parchment, SCHOOLX_PALETTE.terracotta),
  ).toBeCloseTo(3.94, 2);
  expect(
    contrastRatio(SCHOOLX_PALETTE.parchment, SCHOOLX_PALETTE.sage),
  ).toBeCloseTo(2.75, 2);
  expect(contrastRatio(SCHOOLX_PALETTE.pine, SCHOOLX_PALETTE.ink)).toBeCloseTo(
    1.8,
    2,
  );
});

for (const mode of ["light", "dark"] as const) {
  test(`bundle-blocked fresh ${mode} prepaint uses SchoolX from the first document paint`, async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme: mode });
    await seedPreferences(page, {});
    await blockApplicationBundle(page);
    await page.goto("/", { waitUntil: "domcontentloaded" });

    const state = await page.evaluate(() => {
      const root = document.documentElement;
      return {
        background: getComputedStyle(root).backgroundColor,
        backgroundVar: root.style.getPropertyValue("--background").trim(),
        bodyBackground: getComputedStyle(document.body).backgroundColor,
        foregroundVar: root.style.getPropertyValue("--foreground").trim(),
        isDark: root.classList.contains("dark"),
        theme: root.getAttribute("data-buzz-theme"),
      };
    });
    const expected = createSchoolXTheme(mode === "dark" ? "buzz-dark" : "buzz");
    const backing = mode === "dark" ? "rgb(31, 41, 55)" : "rgb(244, 237, 221)";
    expect(state).toEqual({
      background: backing,
      backgroundVar: expected.vars["--background"],
      bodyBackground: backing,
      foregroundVar: expected.vars["--foreground"],
      isDark: expected.isDark,
      theme: mode === "dark" ? "buzz-dark" : "buzz",
    });
  });
}

test("bundle-blocked prepaint rejects an unversioned first-party sentinel cache", async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: "light" });
  await seedPreferences(page, {
    cache: {
      isDark: false,
      themeName: "buzz",
      vars: {
        "--background": "60 100% 50%",
        "--buzz-gradient-top": "sentinel-old-gradient",
      },
    },
    followSystem: "false",
    theme: "buzz",
  });
  await blockApplicationBundle(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });

  const state = await page.evaluate(() => ({
    background: getComputedStyle(document.documentElement).backgroundColor,
    copiedBackground: document.documentElement.style
      .getPropertyValue("--background")
      .trim(),
    copiedGradient: document.documentElement.style
      .getPropertyValue("--buzz-gradient-top")
      .trim(),
    theme: document.documentElement.getAttribute("data-buzz-theme"),
  }));
  expect(state).toEqual({
    background: "rgb(244, 237, 221)",
    copiedBackground: createSchoolXTheme("buzz").vars["--background"],
    copiedGradient: "",
    theme: "buzz",
  });
});

test("bundle-blocked prepaint rejects a mutated current-revision palette", async ({
  page,
}) => {
  const light = createSchoolXTheme("buzz");
  await seedPreferences(page, {
    cache: {
      isDark: false,
      revision: SCHOOLX_THEME_CACHE_REVISION,
      themeName: "buzz",
      vars: { ...light.vars, "--background": "300 100% 50%" },
    },
    followSystem: "false",
    theme: "buzz",
  });
  await blockApplicationBundle(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });

  expect(
    await page.evaluate(() => ({
      background: getComputedStyle(document.documentElement).backgroundColor,
      copiedBackground: document.documentElement.style
        .getPropertyValue("--background")
        .trim(),
      theme: document.documentElement.getAttribute("data-buzz-theme"),
    })),
  ).toEqual({
    background: "rgb(244, 237, 221)",
    copiedBackground: light.vars["--background"],
    theme: "buzz",
  });
});

test("a mounted first-party cache round-trips through revisioned synchronous prepaint", async ({
  page,
}) => {
  await seedPreferences(page, { followSystem: "false", theme: "buzz" });
  await installMockBridge(page);
  await openHome(page);
  await expectAppliedSchoolXTheme(page, "buzz");

  const cache = await page.evaluate((key) => {
    const raw = window.localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as ThemeCachePayload) : null;
  }, CACHE_KEY);
  expect(cache).not.toBeNull();
  expect(cache?.revision).toBe(SCHOOLX_THEME_CACHE_REVISION);
  expect(cache?.themeName).toBe("buzz");
  expect(cache?.vars["--background"]).toBe(
    createSchoolXTheme("buzz").vars["--background"],
  );

  await blockApplicationBundle(page);
  await page.reload({ waitUntil: "domcontentloaded" });
  const prepaint = await page.evaluate(() => ({
    background: document.documentElement.style
      .getPropertyValue("--background")
      .trim(),
    isDark: document.documentElement.classList.contains("dark"),
    revision: JSON.parse(
      window.localStorage.getItem("buzz-theme-cache") ?? "null",
    )?.revision,
    theme: document.documentElement.getAttribute("data-buzz-theme"),
  }));
  expect(prepaint).toEqual({
    background: createSchoolXTheme("buzz").vars["--background"],
    isDark: false,
    revision: SCHOOLX_THEME_CACHE_REVISION,
    theme: "buzz",
  });
});

test("bundle-blocked prepaint skips a valid cache that mismatches stored/system effective mode", async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: "dark" });
  const light = createSchoolXTheme("buzz");
  await seedPreferences(page, {
    cache: {
      isDark: false,
      revision: SCHOOLX_THEME_CACHE_REVISION,
      themeName: "buzz",
      vars: light.vars,
    },
    followSystem: "true",
    theme: "buzz",
  });
  await blockApplicationBundle(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });

  const state = await page.evaluate(() => ({
    background: getComputedStyle(document.documentElement).backgroundColor,
    copiedBackground: document.documentElement.style
      .getPropertyValue("--background")
      .trim(),
    theme: document.documentElement.getAttribute("data-buzz-theme"),
  }));
  expect(state).toEqual({
    background: "rgb(31, 41, 55)",
    copiedBackground: createSchoolXTheme("buzz-dark").vars["--background"],
    theme: "buzz-dark",
  });
});

test("bundle-blocked prepaint rejects an unrelated cache for an unsupported stored ID", async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await seedPreferences(page, {
    cache: {
      isDark: false,
      themeName: "github-light",
      vars: { "--background": "0 0% 100%" },
    },
    followSystem: "true",
    theme: "unsupported-theme",
  });
  await blockApplicationBundle(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });

  expect(
    await page.evaluate(() => ({
      background: getComputedStyle(document.documentElement).backgroundColor,
      theme: document.documentElement.getAttribute("data-buzz-theme"),
    })),
  ).toEqual({
    background: "rgb(31, 41, 55)",
    theme: "buzz-dark",
  });
});

test("invalid cache safely recovers to the stored explicit third-party theme", async ({
  page,
}) => {
  await seedPreferences(page, {
    accent: "#ef4444",
    cache: "{not valid json",
    followSystem: "false",
    theme: "github-light",
  });
  await installMockBridge(page);
  await openHome(page);

  await expect
    .poll(() => readRootThemeState(page))
    .toEqual(
      expect.objectContaining({
        cacheTheme: "github-light",
        isDark: false,
        primary: hexToHsl("#ef4444"),
        storedAccent: "#ef4444",
        storedFollowSystem: "false",
        storedTheme: "github-light",
        theme: null,
        translucent: false,
      }),
    );
});

test("stored/system/legacy/third-party preference matrix preserves effective selection", async ({
  browser,
}) => {
  test.setTimeout(90_000);
  const cases: Array<{
    expectedEffective: string;
    followSystem: string | null;
    label: string;
    system: "dark" | "light";
    theme: string | null;
  }> = [
    {
      label: "fresh",
      theme: null,
      followSystem: null,
      system: "dark",
      expectedEffective: "buzz-dark",
    },
    {
      label: "partial fixed",
      theme: null,
      followSystem: "false",
      system: "dark",
      expectedEffective: "buzz",
    },
    {
      label: "fixed first party",
      theme: "buzz-dark",
      followSystem: "false",
      system: "light",
      expectedEffective: "buzz-dark",
    },
    {
      label: "follow first party",
      theme: "buzz",
      followSystem: "true",
      system: "dark",
      expectedEffective: "buzz-dark",
    },
    {
      label: "explicit third party",
      theme: "github-light",
      followSystem: null,
      system: "dark",
      expectedEffective: "github-light",
    },
    {
      label: "paired third party",
      theme: "catppuccin-latte",
      followSystem: "true",
      system: "dark",
      expectedEffective: "catppuccin-mocha",
    },
    {
      label: "unpaired third party",
      theme: "dracula",
      followSystem: "true",
      system: "light",
      expectedEffective: "dracula",
    },
    {
      label: "legacy light",
      theme: "light",
      followSystem: "false",
      system: "dark",
      expectedEffective: "catppuccin-latte",
    },
    {
      label: "legacy dark",
      theme: "dark",
      followSystem: "false",
      system: "light",
      expectedEffective: "houston",
    },
    {
      label: "legacy system",
      theme: "system",
      followSystem: "true",
      system: "light",
      expectedEffective: "houston",
    },
    {
      label: "invalid fallback",
      theme: "unsupported-theme",
      followSystem: "true",
      system: "dark",
      expectedEffective: "buzz-dark",
    },
  ];

  for (const scenario of cases) {
    const context = await browser.newContext({ colorScheme: scenario.system });
    const page = await context.newPage();
    try {
      await seedPreferences(page, {
        followSystem: scenario.followSystem,
        theme: scenario.theme,
      });
      await installMockBridge(page);
      await openHome(page);
      await expect
        .poll(
          () =>
            page.evaluate((key) => {
              try {
                const raw = window.localStorage.getItem(key);
                return raw
                  ? ((JSON.parse(raw) as { themeName?: string }).themeName ??
                      null)
                  : null;
              } catch {
                return null;
              }
            }, CACHE_KEY),
          { message: scenario.label },
        )
        .toBe(scenario.expectedEffective);
      expect(
        await page.evaluate(
          (key) => localStorage.getItem(key),
          FOLLOW_SYSTEM_KEY,
        ),
      ).toBe(scenario.followSystem);
    } finally {
      await context.close();
    }
  }
});

for (const themeName of ["buzz", "buzz-dark"] as const) {
  test(`1280x720 Home uses flat opaque SchoolX ${themeName === "buzz" ? "Light" : "Dark"}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1280, height: 720 });
    await seedPreferences(page, { followSystem: "false", theme: themeName });
    await installMockBridge(page);
    await openHome(page);
    await expectAppliedSchoolXTheme(page, themeName);
    await expectFlatOpaqueSchoolXShell(page);
    await waitForAnimations(page);
    await page.screenshot({
      path: `${SHOTS}/${themeName === "buzz" ? "01-home-light" : "02-home-dark"}.png`,
    });
  });
}

test("Appearance exposes the SchoolX pair, exact resolver previews, and keyboard selection", async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: "light" });
  await seedPreferences(page, { followSystem: "true", theme: "buzz" });
  await installMockBridge(page);
  await openHome(page);
  const panel = await openAppearance(page, "system");

  const pair = page.getByTestId("theme-pair-buzz");
  await expect(pair).toContainText("SchoolX");
  await expect(pair).toHaveAttribute("aria-pressed", "true");
  await expect(panel.getByText("Buzz", { exact: true })).toHaveCount(0);

  const githubPair = page.getByTestId("theme-pair-github-light");
  await githubPair.focus();
  await expect(githubPair).toBeFocused();
  await githubPair.press("Space");
  await expect(githubPair).toHaveAttribute("aria-pressed", "true");
  await pair.focus();
  await expect(pair).toBeFocused();
  await pair.press("Enter");
  await expect(pair).toHaveAttribute("aria-pressed", "true");
  await expect
    .poll(() => page.evaluate((key) => localStorage.getItem(key), THEME_KEY))
    .toBe("buzz");

  await page.getByTestId("appearance-mode-light").click();
  const lightTile = page.getByTestId("theme-option-buzz");
  await expect(lightTile).toContainText("SchoolX");
  await expectPreviewMatchesRoot(page, lightTile);

  await page.getByTestId("appearance-mode-dark").click();
  const darkTile = page.getByTestId("theme-option-buzz-dark");
  await expect(darkTile).toContainText("SchoolX Dark");
  await expectPreviewMatchesRoot(page, darkTile);
  await expect(panel.getByText("Buzz Dark", { exact: true })).toHaveCount(0);

  await page.getByTestId("appearance-mode-system").click();
  await expect(page.getByTestId("theme-pair-buzz")).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await waitForAnimations(page);
  await panel.screenshot({ path: `${SHOTS}/03-appearance-system.png` });
});

test("first-party accent is fixed while explicit third-party accent and cleanup remain intact", async ({
  page,
}) => {
  await seedPreferences(page, {
    accent: "#ef4444",
    followSystem: "false",
    theme: "buzz",
  });
  await installMockBridge(page);
  await openHome(page);
  await expectAppliedSchoolXTheme(page, "buzz");
  expect((await readRootThemeState(page)).primary).toBe(
    createSchoolXTheme("buzz").vars["--primary"],
  );

  await openAppearance(page, "light");
  await expect(page.getByTestId("accent-color-neutral")).toHaveCount(0);
  await page.getByTestId("theme-option-github-light").click();
  await expect
    .poll(() => readRootThemeState(page))
    .toEqual(
      expect.objectContaining({
        cacheTheme: "github-light",
        primary: hexToHsl("#ef4444"),
        storedAccent: "#ef4444",
        storedTheme: "github-light",
        theme: null,
        translucent: false,
      }),
    );
  const cleanup = await page.evaluate(
    (names) => {
      const root = document.documentElement;
      return {
        firstPartyInlineVars: names.filter(
          (name) => root.style.getPropertyValue(name).trim() !== "",
        ),
        sidebarMarker: root.hasAttribute("data-buzz-sidebar"),
      };
    },
    [...SCHOOLX_THEME_ONLY_VAR_NAMES],
  );
  expect(cleanup).toEqual({ firstPartyInlineVars: [], sidebarMarker: false });
  await expect(page.getByTestId("accent-color-blue")).toBeVisible();
  await page.getByTestId("accent-color-blue").click();
  await expect
    .poll(() => readRootThemeState(page))
    .toEqual(
      expect.objectContaining({
        primary: hexToHsl("#3b82f6"),
        storedAccent: "#3b82f6",
        storedTheme: "github-light",
      }),
    );
});

test("rapid SchoolX Light/Dark selection leaves only the final choice applied", async ({
  page,
}) => {
  await seedPreferences(page, { followSystem: "false", theme: "buzz" });
  await installMockBridge(page);
  await openHome(page);
  await openAppearance(page, "light");
  await page.getByTestId("appearance-mode-dark").click();
  await page.getByTestId("appearance-mode-light").click();
  await page.getByTestId("appearance-mode-dark").click();
  await expectAppliedSchoolXTheme(page, "buzz-dark");
});

test("Follow System reacts to matchMedia changes without reload", async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: "light" });
  await seedPreferences(page, { followSystem: "true", theme: "buzz" });
  await installMockBridge(page);
  await openHome(page);
  await expectAppliedSchoolXTheme(page, "buzz", "buzz");

  await page.emulateMedia({ colorScheme: "dark" });
  await expectAppliedSchoolXTheme(page, "buzz-dark", "buzz");
  await page.emulateMedia({ colorScheme: "light" });
  await expectAppliedSchoolXTheme(page, "buzz", "buzz");
});

test("mocked native vibrancy rejection leaves SchoolX surfaces opaque", async ({
  page,
}) => {
  const vibrancyWarnings: string[] = [];
  page.on("console", (message) => {
    if (
      message.type() === "warning" &&
      message.text().includes("set_window_vibrancy failed")
    ) {
      vibrancyWarnings.push(message.text());
    }
  });
  await seedPreferences(page, { followSystem: "false", theme: "buzz" });
  await page.addInitScript(() => Reflect.set(globalThis, "isTauri", true));
  await installMockBridge(page);
  await openHome(page);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
            ({ command }) => command === "set_window_vibrancy",
          ).length,
      ),
    )
    .toBeGreaterThan(0);
  await expect.poll(() => vibrancyWarnings.length).toBeGreaterThan(0);
  await expectFlatOpaqueSchoolXShell(page);
  await expect(page.locator("html")).not.toHaveAttribute(
    "data-buzz-translucent",
  );
});

test("captures locator-scoped Project card and detail SchoolX Code entry points", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await seedPreferences(page, {
    followSystem: "false",
    theme: "buzz",
    viewMode: "grid",
  });
  await installMockBridge(page, { schoolxCodeWorkspace: true });
  await openHome(page);
  const projectCard = await openProjectsRepository(page);
  await expect(projectCard).toHaveAttribute("data-testid", "project-card-buzz");
  await waitForAnimations(page);
  await projectCard.screenshot({
    path: `${SHOTS}/04-project-card-code-entry.png`,
  });

  await projectCard
    .getByRole("button", { name: "View buzz", exact: true })
    .click();
  const detailCodeAction = page.getByRole("button", {
    name: "Open buzz in SchoolX Code",
    exact: true,
  });
  await expect(detailCodeAction).toBeVisible();
  const detailActionGroup = detailCodeAction.locator("xpath=..");
  await waitForAnimations(page);
  await detailActionGroup.screenshot({
    path: `${SHOTS}/05-project-detail-code-entry.png`,
  });
});

test("800x500 Settings at 150% keeps theme CTAs visible and keyboard reachable", async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 500 });
  await seedPreferences(page, {
    followSystem: "false",
    textScale: 1.5,
    theme: "buzz-dark",
  });
  await installMockBridge(page);
  await openHome(page);
  await openAppearance(page, "dark");
  await expect
    .poll(() =>
      page.evaluate(() => getComputedStyle(document.documentElement).fontSize),
    )
    .toBe("24px");
  const systemMode = page.getByTestId("appearance-mode-system");
  await expectInsideViewport(page, systemMode);
  await systemMode.focus();
  await expect(systemMode).toBeFocused();
  await systemMode.press("Enter");
  await expect(systemMode).toHaveAttribute("aria-pressed", "true");
  const pair = page.getByTestId("theme-pair-buzz");
  await expectInsideViewport(page, pair);
  await pair.focus();
  expect(
    await pair
      .locator(":scope > div")
      .evaluate((element) => getComputedStyle(element).boxShadow),
  ).not.toBe("none");
  await pair.press("Space");
  await expect(pair).toHaveAttribute("aria-pressed", "true");
  await expectNoDocumentOverflow(page);
});

test("800x500 Projects at 150% keeps the SchoolX Code CTA visible and keyboard reachable", async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 500 });
  await seedPreferences(page, {
    followSystem: "false",
    textScale: 1.5,
    theme: "buzz-dark",
    viewMode: "grid",
  });
  await installMockBridge(page, { schoolxCodeWorkspace: true });
  await openHome(page);
  const projectEntry = await openProjectsRepository(page);
  const action = projectEntry.getByRole("button", {
    name: "Open buzz in SchoolX Code",
    exact: true,
  });
  await expectInsideViewport(page, action);
  await expectNoDocumentOverflow(page);
  await action.focus();
  await expect(action).toBeFocused();
  await action.press("Enter");
  await expect(page).toHaveURL(/\/#\/projects\/[^/]+\/code(?:\?|$)/);
});

test("800x500 SchoolX Code Dark at 150% remains keyboard-operable with reduced motion", async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 500 });
  await page.emulateMedia({ colorScheme: "dark", reducedMotion: "reduce" });
  await seedPreferences(page, {
    followSystem: "false",
    textScale: 1.5,
    theme: "buzz-dark",
    viewMode: "grid",
  });
  await installMockBridge(page, { schoolxCodeWorkspace: true });
  await openHome(page);
  await openCodeWorkspace(page, true);
  const thread = page.getByTestId(`code-thread-${CODE_THREAD_ID}`);
  await thread.focus();
  await expect(thread).toBeFocused();
  await thread.press("Enter");
  const newTask = page.getByRole("button", { name: "New task", exact: true });
  await expectInsideViewport(page, newTask);
  await newTask.focus();
  await expect(newTask).toBeFocused();
  await newTask.press("Enter");
  await expect(page.getByTestId("code-new-task-dialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("code-new-task-dialog")).toBeHidden();
  const hideSidebar = page.getByRole("button", { name: "Hide task sidebar" });
  await hideSidebar.focus();
  await hideSidebar.press("Enter");
  await expect(
    page.getByRole("heading", { name: "Code shell task" }),
  ).toBeVisible();
  await expectAppliedSchoolXTheme(page, "buzz-dark");
  await expectNoDocumentOverflow(page);
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/06-code-dark-reduced-150.png` });
});
