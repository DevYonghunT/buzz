import type { Page, Route } from "@playwright/test";

async function fetchStaticModule(route: Route) {
  for (let attempt = 0; ; attempt += 1) {
    try {
      return await route.fetch({ maxRetries: 2 });
    } catch (error) {
      const isBrokenPipe =
        error instanceof Error && /\bEPIPE\b/.test(error.message);
      if (!isBrokenPipe || attempt >= 2) throw error;
      await new Promise((resolve) => setTimeout(resolve, 50 * 2 ** attempt));
    }
  }
}

export async function openMockApp(page: Page) {
  // Python's preview server can reset some of Vite's large parallel ESM fan-out
  // in a cold browser context. Serialize only local built modules and retry
  // transport resets; product bootstrap/render failures still fail below.
  let moduleQueue = Promise.resolve();

  await page.route("http://127.0.0.1:4173/assets/*.js", async (route) => {
    const response = moduleQueue.then(() => fetchStaticModule(route));
    moduleQueue = response.then(
      () => undefined,
      () => undefined,
    );
    await route.fulfill({ response: await response });
  });

  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.waitForFunction(
    () => Array.isArray(window.__BUZZ_E2E_COMMANDS__),
    undefined,
    { timeout: 15_000 },
  );
}

export async function cleanupMockAppRoutes(page: Page) {
  if (!page.isClosed()) {
    await page.unrouteAll({ behavior: "ignoreErrors" });
  }
}
