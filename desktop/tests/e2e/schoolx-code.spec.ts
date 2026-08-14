import { expect, test, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const REPOSITORY_IDENTITY = "c4".repeat(32);
const THREAD_ID = "thread-e2e-1";
const PROMPT = "Please verify the focused SchoolX Code UI shell.";
const STEER_PROMPT = "Also report the focused validation result.";
const SCOPE = {
  communityId: "e2e-default-community",
  projectDtag: "buzz",
  repositoryIdentity: REPOSITORY_IDENTITY,
};

async function commandPayload(page: Page, command: string) {
  return page.evaluate((requestedCommand) => {
    const calls = (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
      (entry) => entry.command === requestedCommand,
    );
    return calls.at(-1)?.payload;
  }, command);
}

async function waitForCommand(page: Page, command: string) {
  await expect
    .poll(() =>
      page.evaluate(
        (requestedCommand) =>
          (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).some(
            (entry) => entry.command === requestedCommand,
          ),
        command,
      ),
    )
    .toBe(true);
}

async function enterMockApp(page: Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  try {
    await page.waitForFunction(
      () => Array.isArray(window.__BUZZ_E2E_COMMANDS__),
      undefined,
      { timeout: 5_000 },
    );
  } catch {
    // A reset static-module request leaves the mock bridge uninstalled. One
    // fresh navigation recovers without masking app-level render failures.
    await page.reload({ waitUntil: "domcontentloaded" });
    await page.waitForFunction(
      () => Array.isArray(window.__BUZZ_E2E_COMMANDS__),
      undefined,
      { timeout: 5_000 },
    );
  }
}

test("opens a scoped SchoolX Code task and submits through its bound thread", async ({
  page,
}) => {
  await installMockBridge(page, { schoolxCodeWorkspace: true });

  // The preview server has no SPA fallback, so enter through the app shell.
  await enterMockApp(page);
  await page.getByTestId("open-projects-view").click();
  await page.getByRole("button", { name: "Repositories", exact: true }).click();

  const projectEntry = page
    .locator(
      '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
    )
    .first();
  await expect(projectEntry).toBeVisible({ timeout: 10_000 });
  await projectEntry.click();

  await page.getByRole("button", { name: "Code", exact: true }).click();
  await expect(page).toHaveURL(/\/#\/projects\/[^/]+\/code(?:\?|$)/);
  await expect(
    page.getByRole("navigation", { name: "Code project breadcrumb" }),
  ).toContainText("buzz");

  await expect(page.getByText("Ready", { exact: true })).toBeVisible({
    timeout: 10_000,
  });
  await waitForCommand(page, "code_runtime_status");
  await waitForCommand(page, "code_runtime_start");
  await waitForCommand(page, "code_runtime_events");
  await waitForCommand(page, "code_thread_preparations_list");
  await waitForCommand(page, "code_threads_list");

  expect(await commandPayload(page, "code_repository_inspect")).toEqual({
    input: { repositoryRoot: "/mock/buzz", baseRef: "main" },
  });
  expect(await commandPayload(page, "code_runtime_status")).toEqual({});
  expect(await commandPayload(page, "code_runtime_start")).toEqual({});
  expect(await commandPayload(page, "code_runtime_events")).toEqual({
    scope: SCOPE,
    runtimeGeneration: 7,
    afterSequence: 0,
  });
  expect(await commandPayload(page, "code_thread_preparations_list")).toEqual({
    input: { scope: SCOPE },
  });
  expect(await commandPayload(page, "code_threads_list")).toEqual({
    input: { scope: SCOPE },
  });
  await expect(
    page.getByTestId("code-preparation-preparation-e2e-existing"),
  ).toContainText("Prepared task");

  const thread = page.getByTestId(`code-thread-${THREAD_ID}`);
  await expect(thread).toBeVisible();
  await thread.click();

  await expect(page).toHaveURL(new RegExp(`threadId=${THREAD_ID}`));
  await waitForCommand(page, "code_thread_resume");
  expect(await commandPayload(page, "code_thread_resume")).toEqual({
    input: {
      scope: SCOPE,
      threadId: THREAD_ID,
      model: null,
    },
  });
  await expect(page.getByTestId("code-timeline")).toContainText(
    "Historical Code response from fixture.",
  );

  const composer = page.getByRole("textbox", { name: "Message Code task" });
  await expect(composer).toBeEnabled();
  await composer.fill(PROMPT);
  await page.getByRole("button", { name: "Send prompt" }).click();

  await expect(page.getByTestId("code-timeline")).toContainText(PROMPT);
  await waitForCommand(page, "code_turn_start");
  expect(await commandPayload(page, "code_turn_start")).toEqual({
    input: {
      scope: SCOPE,
      threadId: THREAD_ID,
      prompt: PROMPT,
      model: null,
      effort: null,
    },
  });

  const steeringComposer = page.getByRole("textbox", {
    name: "Steer active Code task",
  });
  await expect(steeringComposer).toBeEnabled();
  await steeringComposer.fill(STEER_PROMPT);
  await page.getByRole("button", { name: "Steer active turn" }).click();

  await waitForCommand(page, "code_turn_steer");
  expect(await commandPayload(page, "code_turn_steer")).toEqual({
    input: {
      scope: SCOPE,
      threadId: THREAD_ID,
      expectedTurnId: "turn-e2e-new",
      prompt: STEER_PROMPT,
    },
  });
});
