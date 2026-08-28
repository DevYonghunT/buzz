import { expect, test, type Locator, type Page } from "@playwright/test";

import type {
  CodeEventBacklog,
  CodeEventCheckpoint,
  CodeRequestId,
  CodeWorkspaceEvent,
  CodeWorkspaceEventKind,
  JsonValue,
} from "../../src/features/code/api/types";
import type {
  CodeGitCommitReceipt,
  CodeGitIndexMutationReceipt,
} from "../../src/features/code/api/codeGitTypes";

import { installMockBridge } from "../helpers/bridge";
import {
  cleanupMockAppRoutes,
  openMockApp as enterMockApp,
} from "../helpers/mockApp";
import {
  SCHOOLX_CODE_SCOPE as SCOPE,
  SCHOOLX_CREATED_THREAD_ID as CREATED_THREAD_ID,
  FRESH_THREAD_CHANGES,
  PARTIAL_THREAD_CHANGES,
  STALE_THREAD_CHANGES,
  gitReadyStatus,
} from "../helpers/schoolxCodeFixtures";

const THREAD_ID = "thread-e2e-1";
const FORKED_THREAD_ID = "thread-e2e-forked";
const CREATED_PREPARATION_ID = "preparation-e2e-new";
const FORK_PREPARATION_ID = "preparation-e2e-fork";
const CREATED_TURN_ID = "turn-e2e-new";
const TURN_ID = "turn-e2e-normalized";
const PROMPT = "Please verify the focused SchoolX Code UI shell.";
const SELECTOR_PROMPT =
  "Verify the persisted model and reasoning selection payload.";
const CATALOG_FALLBACK_PROMPT =
  "Continue with the Codex defaults while the catalog is unavailable.";
const STEER_PROMPT = "Also report the focused validation result.";
const INTERRUPT_PROMPT = "Stop after the normalized turn begins.";
const PREPARATION_SCOPE_ERROR =
  "SchoolX Code preparation was not found in the requested scope";
const THREAD_SCOPE_ERROR =
  "Codex thread is not bound to the requested SchoolX community, project, and repository";
const PREPARATION_SCOPE_FAILURE = {
  message: PREPARATION_SCOPE_ERROR,
  payload: {
    code: "preparationUnavailable",
    message: PREPARATION_SCOPE_ERROR,
    preparationId: null,
    threadId: null,
    executionRoot: null,
  },
};
const THREAD_SCOPE_FAILURE = { message: THREAD_SCOPE_ERROR, payload: null };
function codeEvent(
  sequence: number,
  kind: CodeWorkspaceEventKind,
  options: {
    itemId?: string | null;
    payload?: JsonValue;
    runtimeGeneration?: number;
    threadId?: string | null;
    turnId?: string | null;
  } = {},
): CodeWorkspaceEvent {
  return {
    scope: SCOPE,
    runtimeGeneration: options.runtimeGeneration ?? 7,
    sequence,
    threadId: options.threadId === undefined ? THREAD_ID : options.threadId,
    turnId: options.turnId === undefined ? TURN_ID : options.turnId,
    itemId: options.itemId ?? null,
    kind,
    payload: options.payload ?? {},
  };
}

function approvalEvent({
  approvalKind,
  itemId,
  kind,
  request,
  requestId,
  sequence,
}: {
  approvalKind: "commandExecution" | "fileChange" | "permissions";
  itemId: string;
  kind:
    | "item/commandExecution/requestApproval"
    | "item/fileChange/requestApproval"
    | "item/permissions/requestApproval";
  request: Record<string, JsonValue>;
  requestId: CodeRequestId;
  sequence: number;
}): CodeWorkspaceEvent {
  return codeEvent(sequence, kind, {
    itemId,
    payload: {
      requestId,
      approvalKind,
      request: {
        threadId: THREAD_ID,
        turnId: TURN_ID,
        itemId,
        ...request,
      },
    },
  });
}

function eventBacklog(
  events: CodeWorkspaceEvent[],
  truncated = false,
  runtimeGeneration = events.at(-1)?.runtimeGeneration ?? 7,
  checkpoint: CodeEventCheckpoint | null = null,
): CodeEventBacklog {
  return {
    runtimeGeneration,
    latestSequence: events.at(-1)?.sequence ?? 0,
    truncated,
    checkpoint,
    events,
  };
}

const COMMAND_APPROVAL = approvalEvent({
  approvalKind: "commandExecution",
  itemId: "approval-command",
  kind: "item/commandExecution/requestApproval",
  request: {
    command: "git status --short",
    cwd: "/mock/buzz",
    reason: "Confirm the focused repository state.",
  },
  requestId: 41,
  sequence: 8,
});

const FILE_APPROVAL = approvalEvent({
  approvalKind: "fileChange",
  itemId: "approval-file",
  kind: "item/fileChange/requestApproval",
  request: { reason: "Apply the normalized file update." },
  requestId: "41",
  sequence: 9,
});

const PERMISSION_DECLINE_APPROVAL = approvalEvent({
  approvalKind: "permissions",
  itemId: "approval-permission-decline",
  kind: "item/permissions/requestApproval",
  request: {
    cwd: "/mock/buzz",
    permissionDisplay: {
      grantable: true,
      network: { enabled: true },
      fileSystem: null,
    },
    reason: "Decline the standalone network permission request.",
  },
  requestId: "permission-decline",
  sequence: 10,
});

const PERMISSION_TURN_APPROVAL = approvalEvent({
  approvalKind: "permissions",
  itemId: "approval-permission-turn",
  kind: "item/permissions/requestApproval",
  request: {
    cwd: "/mock/buzz",
    permissionDisplay: {
      grantable: true,
      network: null,
      fileSystem: {
        entries: null,
        globScanMaxDepth: null,
        read: ["/mock/buzz"],
        write: null,
      },
    },
    reason: "Allow this read permission for the current turn.",
  },
  requestId: 42,
  sequence: 11,
});

const PERMISSION_SESSION_APPROVAL = approvalEvent({
  approvalKind: "permissions",
  itemId: "approval-permission-session",
  kind: "item/permissions/requestApproval",
  request: {
    cwd: "/mock/buzz",
    permissionDisplay: {
      grantable: true,
      network: null,
      fileSystem: {
        entries: null,
        globScanMaxDepth: null,
        read: null,
        write: ["/mock/buzz"],
      },
    },
    reason: "Allow this write permission for the session.",
  },
  requestId: "permission-session",
  sequence: 12,
});

const NORMALIZED_TIMELINE_EVENTS = [
  codeEvent(1, "turn/started", {
    payload: { turn: { id: TURN_ID, status: "inProgress" } },
  }),
  codeEvent(2, "turn/plan/updated", {
    payload: {
      explanation: "Verify the normalized Code event pipeline.",
      plan: [
        { step: "Inspect the focused adapter", status: "completed" },
        { step: "Run the focused E2E", status: "inProgress" },
      ],
    },
  }),
  codeEvent(3, "item/started", {
    itemId: "item-command",
    payload: {
      item: {
        id: "item-command",
        type: "commandExecution",
        command: ["pnpm", "test:e2e", "schoolx-code"],
        aggregatedOutput: "",
        status: "inProgress",
      },
    },
  }),
  codeEvent(4, "item/commandExecution/outputDelta", {
    itemId: "item-command",
    payload: { delta: "3 focused tests passed\n" },
  }),
  codeEvent(5, "item/completed", {
    itemId: "item-command",
    payload: {
      item: {
        id: "item-command",
        type: "commandExecution",
        command: ["pnpm", "test:e2e", "schoolx-code"],
        aggregatedOutput: "3 focused tests passed\n",
        status: "completed",
        exitCode: 0,
      },
    },
  }),
  codeEvent(6, "item/started", {
    itemId: "item-file",
    payload: {
      item: {
        id: "item-file",
        type: "fileChange",
        status: "inProgress",
        changes: [
          {
            path: "desktop/src/features/code/ui/CodeTimeline.tsx",
            kind: { type: "update" },
          },
        ],
      },
    },
  }),
  codeEvent(7, "item/completed", {
    itemId: "item-file",
    payload: {
      item: {
        id: "item-file",
        type: "fileChange",
        status: "completed",
        changes: [
          {
            path: "desktop/src/features/code/ui/CodeTimeline.tsx",
            kind: { type: "update" },
          },
        ],
      },
    },
  }),
  COMMAND_APPROVAL,
  FILE_APPROVAL,
  PERMISSION_DECLINE_APPROVAL,
  PERMISSION_TURN_APPROVAL,
  PERMISSION_SESSION_APPROVAL,
];

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

async function waitForCommandCount(page: Page, command: string, count: number) {
  await expect
    .poll(() =>
      page.evaluate(
        (requestedCommand) =>
          (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
            (entry) => entry.command === requestedCommand,
          ).length,
        command,
      ),
    )
    .toBe(count);
}

async function commandPayloads(page: Page, command: string) {
  return page.evaluate(
    (requestedCommand) =>
      (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [])
        .filter((entry) => entry.command === requestedCommand)
        .map((entry) => entry.payload),
    command,
  );
}

async function invokeMockCommand(
  page: Page,
  command: string,
  payload: Record<string, unknown>,
) {
  return page.evaluate(
    async ({ requestedCommand, requestedPayload }) => {
      const invoke = window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
      if (!invoke) throw new Error("Mock Tauri command seam is unavailable");
      return invoke(requestedCommand, requestedPayload);
    },
    { requestedCommand: command, requestedPayload: payload },
  );
}

async function invokeMockCommandError(
  page: Page,
  command: string,
  payload: Record<string, unknown>,
) {
  return page.evaluate(
    async ({ requestedCommand, requestedPayload }) => {
      const invoke = window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
      if (!invoke) return "Mock Tauri command seam is unavailable";
      try {
        await invoke(requestedCommand, requestedPayload);
        return null;
      } catch (error) {
        return {
          message: error instanceof Error ? error.message : String(error),
          payload:
            typeof error === "object" && error !== null && "payload" in error
              ? error.payload
              : null,
        };
      }
    },
    { requestedCommand: command, requestedPayload: payload },
  );
}

async function activate(page: Page, target: Locator, keyboardOnly: boolean) {
  await expect(target).toBeVisible();
  if (!keyboardOnly) {
    await target.click();
    return;
  }
  await target.focus();
  await expect(target).toBeFocused();
  await page.keyboard.press("Enter");
}

test.afterEach(async ({ page }) => {
  await cleanupMockAppRoutes(page);
});

function codeRuntimeReadyLabel(page: Page) {
  return page.getByRole("paragraph").filter({ hasText: /^Ready$/ });
}

async function targetedProjectLookupCount(page: Page) {
  return page.evaluate(
    () =>
      window.__BUZZ_E2E_PROJECT_QUERY_FILTERS__?.filter(
        (filter) =>
          filter.kinds?.length === 1 &&
          filter.kinds[0] === 30617 &&
          filter["#d"]?.includes("buzz"),
      ).length ?? 0,
  );
}

async function openCodeProjectRoute(page: Page, keyboardOnly = false) {
  await enterMockApp(page);
  await activate(page, page.getByTestId("open-projects-view"), keyboardOnly);
  await activate(
    page,
    page.getByRole("button", { name: "Repositories", exact: true }),
    keyboardOnly,
  );

  const projectEntry = page
    .locator(
      '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
    )
    .first();
  await expect(projectEntry).toBeVisible({ timeout: 10_000 });
  await activate(
    page,
    projectEntry.getByRole("button", {
      name: "Open buzz in SchoolX Code",
      exact: true,
    }),
    keyboardOnly,
  );
  await expect(page).toHaveURL(/\/#\/projects\/[^/]+\/code(?:\?|$)/);
  await expect(
    page.getByRole("navigation", { name: "Code project breadcrumb" }),
  ).toContainText("buzz");
}

async function expectProminentCodeAction(action: Locator) {
  const styles = await action.evaluate((element) => {
    const probe = document.createElement("span");
    probe.style.backgroundColor = "hsl(var(--primary))";
    probe.style.color = "hsl(var(--primary-foreground))";
    document.body.append(probe);

    const actionStyles = getComputedStyle(element);
    const primaryStyles = getComputedStyle(probe);
    const result = {
      backgroundColor: actionStyles.backgroundColor,
      color: actionStyles.color,
      height: element.getBoundingClientRect().height,
      primaryBackgroundColor: primaryStyles.backgroundColor,
      primaryForegroundColor: primaryStyles.color,
    };
    probe.remove();
    return result;
  });

  expect(styles.backgroundColor).toBe(styles.primaryBackgroundColor);
  expect(styles.color).toBe(styles.primaryForegroundColor);
  expect(styles.height).toBeGreaterThanOrEqual(36);
}

test("keeps SchoolX Code entry points visible across narrow project layouts", async ({
  page,
}) => {
  await page.setViewportSize({ height: 600, width: 768 });
  await page.addInitScript(() => {
    window.localStorage.setItem("buzz.projects.viewMode", "grid");
  });
  await installMockBridge(page, { schoolxCodeWorkspace: true });

  await enterMockApp(page);
  await page.getByTestId("open-projects-view").click();
  await page.getByRole("button", { name: "Repositories", exact: true }).click();
  const projectEntry = page
    .locator(
      '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
    )
    .first();
  const projectCodeAction = projectEntry.getByRole("button", {
    name: "Open buzz in SchoolX Code",
    exact: true,
  });
  await expectProminentCodeAction(projectCodeAction);
  for (const width of [768, 900]) {
    await page.setViewportSize({ height: 600, width });
    await expect(projectCodeAction).toBeVisible();
    const projectBox = await projectEntry.boundingBox();
    const actionBox = await projectCodeAction.boundingBox();
    expect(projectBox).not.toBeNull();
    expect(actionBox).not.toBeNull();
    expect(actionBox?.x ?? -1).toBeGreaterThanOrEqual(projectBox?.x ?? 0);
    expect(actionBox?.y ?? -1).toBeGreaterThanOrEqual(projectBox?.y ?? 0);
    expect((actionBox?.x ?? 0) + (actionBox?.width ?? 0)).toBeLessThanOrEqual(
      (projectBox?.x ?? 0) + (projectBox?.width ?? 0),
    );
    expect((actionBox?.y ?? 0) + (actionBox?.height ?? 0)).toBeLessThanOrEqual(
      (projectBox?.y ?? 0) + (projectBox?.height ?? 0),
    );
  }
  await activate(
    page,
    projectEntry.getByRole("button", { name: "View buzz", exact: true }),
    true,
  );

  const codeAction = page.getByRole("button", {
    name: "Open buzz in SchoolX Code",
    exact: true,
  });
  await expect(codeAction).toContainText("SchoolX Code");
  await expectProminentCodeAction(codeAction);
  const terminalAction = page.getByRole("button", {
    name: "Open terminal",
    exact: true,
  });
  for (const width of [768, 900]) {
    await page.setViewportSize({ height: 600, width });
    for (const action of [codeAction, terminalAction]) {
      await expect(action).toBeVisible();
      const actionBox = await action.boundingBox();
      expect(actionBox).not.toBeNull();
      expect(actionBox?.x ?? -1).toBeGreaterThanOrEqual(0);
      expect((actionBox?.x ?? 0) + (actionBox?.width ?? 0)).toBeLessThanOrEqual(
        width,
      );
    }
  }
  await codeAction.click();
  await expect(page).toHaveURL(/\/#\/projects\/[^/]+\/code(?:\?|$)/);
  await expect(
    page.getByRole("navigation", { name: "Code project breadcrumb" }),
  ).toContainText("buzz");
});

async function openCodeWorkspace(page: Page, keyboardOnly = false) {
  await openCodeProjectRoute(page, keyboardOnly);
  await expect(codeRuntimeReadyLabel(page)).toBeVisible({
    timeout: 10_000,
  });
}

async function openBoundThread(page: Page, keyboardOnly = false) {
  await openCodeWorkspace(page, keyboardOnly);
  const thread = page.getByTestId(`code-thread-${THREAD_ID}`);
  await activate(page, thread, keyboardOnly);
  await expect(page).toHaveURL(new RegExp(`threadId=${THREAD_ID}`));
  await waitForCommand(page, "code_thread_resume");
  await expect(page.getByTestId("code-timeline")).toContainText(
    "Historical Code response from fixture.",
  );
}

async function emitCodeEvent(page: Page, event: CodeWorkspaceEvent) {
  await page.evaluate(async (normalizedEvent) => {
    const emitEvent = window.__BUZZ_E2E_EMIT_CODE_WORKSPACE_EVENT__;
    if (!emitEvent) throw new Error("Code workspace event seam is unavailable");
    await emitEvent(normalizedEvent);
  }, event);
}

async function waitForTwoAnimationFrames(page: Page) {
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
      }),
  );
}

async function createManagedTask(
  page: Page,
  expectedModel: string | null = "gpt-5.2-codex",
) {
  const threadListCount = (await commandPayloads(page, "code_threads_list"))
    .length;
  const newTask = page.getByRole("button", { name: "New task", exact: true });
  await expect(newTask).toBeEnabled();
  await newTask.click();
  const dialog = page.getByRole("dialog", { name: "New Code task" });
  const managedMode = dialog.getByRole("radio", {
    name: /Managed worktree/,
  });
  const localMode = dialog.getByRole("radio", { name: /Local checkout/ });
  await expect(managedMode).toBeChecked();
  await expect(localMode).not.toBeChecked();
  await dialog.getByRole("button", { name: "Create task" }).click();

  await waitForCommand(page, "code_worktree_prepare");
  expect(await commandPayload(page, "code_worktree_prepare")).toEqual({
    input: {
      scope: SCOPE,
      repositoryRoot: "/mock/buzz",
      baseRef: "main",
      executionMode: "worktree",
    },
  });

  await waitForCommand(page, "code_thread_start");
  expect(await commandPayload(page, "code_thread_start")).toEqual({
    input: {
      scope: SCOPE,
      preparationId: CREATED_PREPARATION_ID,
      model: expectedModel,
    },
  });

  await expect(page).toHaveURL(new RegExp(`threadId=${CREATED_THREAD_ID}`));
  const createdThread = page.getByTestId(`code-thread-${CREATED_THREAD_ID}`);
  await expect(createdThread).toBeVisible();
  await expect(createdThread).toHaveAttribute("aria-current", "page");
  await expect(createdThread).toContainText("Managed worktree");
  await expect(
    page.getByRole("heading", { name: "Managed worktree task", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("textbox", { name: "Message Code task" }),
  ).toBeEnabled();
  await expect(
    page.getByTestId(`code-thread-lifecycle-${CREATED_THREAD_ID}`),
  ).toHaveText("Active");
  expect((await commandPayloads(page, "code_threads_list")).length).toBe(
    threadListCount,
  );
  await waitForTwoAnimationFrames(page);
  expect(await commandPayloads(page, "code_thread_resume")).toEqual([
    {
      input: {
        scope: SCOPE,
        threadId: THREAD_ID,
        model: null,
      },
    },
  ]);
}

test("refreshes a missing checkout even when Terminal reports an existing clone", async ({
  page,
}) => {
  await installMockBridge(page, {
    projectOmitCloneTag: true,
    schoolxCodeWorkspace: true,
    schoolxCodeHasLocalCheckout: false,
    schoolxCodeTerminalReportsCloned: false,
  });

  await openCodeProjectRoute(page);

  await expect(
    page.getByRole("heading", { name: "Local checkout required" }),
  ).toBeVisible();
  const cloneAction = page.getByRole("button", {
    name: "Clone & open in Terminal",
    exact: true,
  });
  await expect(cloneAction).toBeVisible();

  await cloneAction.click();
  await waitForCommand(page, "open_project_terminal");
  expect(await commandPayload(page, "open_project_terminal")).toEqual({
    reposDir: null,
    projectDtag: "buzz",
    cloneUrl: expect.stringMatching(
      /^http:\/\/localhost:3000\/git\/[0-9a-f]{64}\/buzz$/,
    ),
    defaultBranch: "main",
  });
  await expect(codeRuntimeReadyLabel(page)).toBeVisible({ timeout: 10_000 });
  await expect(
    page.getByRole("heading", { name: "Local checkout required" }),
  ).toHaveCount(0);
});

test("uses the authoritative published branch when Code opens from the project list", async ({
  page,
}) => {
  await page.addInitScript(() => {
    window.sessionStorage.setItem(
      "buzz-e2e-project-branches",
      JSON.stringify({
        buzz: {
          master: "0123456789abcdef0123456789abcdef01234567",
        },
      }),
    );
  });
  await installMockBridge(page, {
    projectHeadBranch: "master",
    schoolxCodeWorkspace: true,
  });

  await openCodeProjectRoute(page);

  await expect(codeRuntimeReadyLabel(page)).toBeVisible({ timeout: 10_000 });
  expect(await commandPayload(page, "code_repository_inspect")).toEqual({
    input: { repositoryRoot: "/mock/buzz", baseRef: "master" },
  });
  await expect(
    page.getByRole("heading", { name: "Project scope unavailable" }),
  ).toHaveCount(0);
});

test("keeps a failed checkout clone retryable in SchoolX Code", async ({
  page,
}) => {
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeHasLocalCheckout: false,
    schoolxCodeTerminalError: "Mock checkout failed",
  });

  await openCodeProjectRoute(page);
  const cloneAction = page.getByRole("button", {
    name: "Clone & open in Terminal",
    exact: true,
  });
  await cloneAction.click();
  await waitForCommand(page, "open_project_terminal");

  await expect(
    page
      .locator("[data-sonner-toast]")
      .filter({ hasText: "Mock checkout failed" }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Local checkout required" }),
  ).toBeVisible();
  await expect(cloneAction).toBeEnabled();
});

test("explains that an empty repository needs its first commit", async ({
  page,
}) => {
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeEmptyLocalRepository: true,
  });

  await openCodeProjectRoute(page);

  await expect(
    page.getByRole("heading", { name: "First commit required" }),
  ).toBeVisible();
  await expect(page.getByText(/create its first commit/i)).toBeVisible();
  const terminalAction = page.getByRole("button", {
    name: "Open in Terminal",
    exact: true,
  });
  await expect(terminalAction).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Retry", exact: true }),
  ).toBeVisible();

  await terminalAction.click();
  await waitForCommand(page, "open_project_terminal");
  expect(await commandPayload(page, "open_project_terminal")).toEqual({
    reposDir: null,
    projectDtag: "buzz",
    cloneUrl: expect.stringMatching(/\/buzz$/),
    defaultBranch: "main",
  });
  await expect(
    page.getByRole("heading", { name: "First commit required" }),
  ).toBeVisible();

  await page.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "First commit required" }),
  ).toBeVisible();
});

test("keeps unrelated repository inspection failures generic", async ({
  page,
}) => {
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeRepositoryInspectError:
      "SchoolX Code repository root is not a Git repository",
  });

  await openCodeProjectRoute(page);

  await expect(
    page.getByRole("heading", { name: "Project scope unavailable" }),
  ).toBeVisible();
  await expect(
    page.getByText(
      "The local repository or selected base branch could not be validated.",
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Retry", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Open project", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("heading", { name: "First commit required" }),
  ).toHaveCount(0);
});

test("explains the permanent macOS signing requirement without retrying", async ({
  page,
}) => {
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeRepositoryInspectError:
      "SchoolX Code repository inspection failed: SchoolX Code Git requires a Developer ID signed SchoolX application",
  });

  await openCodeProjectRoute(page);

  const signedAppAlert = page.getByRole("alert");
  await expect(signedAppAlert).toBeVisible();
  await expect(
    signedAppAlert.getByRole("heading", {
      name: "Signed SchoolX app required",
    }),
  ).toBeVisible();
  await expect(
    signedAppAlert.getByText(
      "SchoolX Code on macOS requires a signed and notarized SchoolX app installed in Applications. Quit this copy, install the signed app in Applications, then open SchoolX again.",
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Retry", exact: true }),
  ).toHaveCount(0);

  const backToProject = signedAppAlert.getByRole("button", {
    name: "Back to Project",
    exact: true,
  });
  await expect(backToProject).toBeVisible();
  await backToProject.click();
  await expect(page).toHaveURL(/\/#\/projects\/[^/?]+$/);
});

test("retries a transient project lookup without treating the project as absent", async ({
  page,
}) => {
  await installMockBridge(page, {
    projectQueryDelayMs: 300,
    schoolxCodeWorkspace: true,
  });

  await enterMockApp(page);
  await page.getByTestId("open-projects-view").click();
  await page.getByRole("button", { name: "Repositories", exact: true }).click();
  const projectEntry = page
    .locator(
      '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
    )
    .first();
  await expect(projectEntry).toBeVisible();
  await page.evaluate(() => {
    window.__BUZZ_E2E_REJECT_PROJECT_QUERY_KINDS__ = [30617];
  });
  await projectEntry
    .getByRole("button", {
      name: "Open buzz in SchoolX Code",
      exact: true,
    })
    .click();

  await expect(
    page.getByRole("heading", { name: "Project load failed", exact: true }),
  ).toBeVisible({ timeout: 10_000 });
  await expect(page.getByText("A relay request failed.")).toBeVisible();
  const retry = page.getByRole("button", { name: "Retry", exact: true });
  await expect(retry).toBeEnabled();
  await expect.poll(() => targetedProjectLookupCount(page)).toBe(2);

  await page.evaluate(() => {
    window.__BUZZ_E2E_REJECT_PROJECT_QUERY_KINDS__ = [];
  });
  await retry.click();
  await expect(
    page
      .getByRole("main")
      .getByRole("status")
      .filter({ hasText: "Opening SchoolX Code" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Retry", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("navigation", { name: "Code project breadcrumb" }),
  ).toContainText("buzz");
  await expect.poll(() => targetedProjectLookupCount(page)).toBe(3);
  await expect(codeRuntimeReadyLabel(page)).toBeVisible({ timeout: 10_000 });

  const missingProjectId = `${"a".repeat(64)}:missing-project`;
  await page.evaluate((projectId) => {
    window.location.hash = `/projects/${encodeURIComponent(projectId)}/code`;
  }, missingProjectId);
  await expect(
    page.getByRole("heading", { name: "Project unavailable", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText(
      "This project is no longer available in the active community.",
      { exact: true },
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Retry", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Back to Projects", exact: true }),
  ).toBeVisible();
});

test("opens a scoped SchoolX Code task and submits through its bound thread", async ({
  page,
}) => {
  await installMockBridge(page, { schoolxCodeWorkspace: true });

  // The preview server has no SPA fallback, so enter through the app shell.
  await openCodeWorkspace(page);
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

  await activate(page, page.getByTestId(`code-thread-${THREAD_ID}`), false);
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

  await waitForCommand(page, "code_thread_changes");
  expect(await commandPayload(page, "code_thread_changes")).toEqual({
    input: { scope: SCOPE, threadId: THREAD_ID },
  });
  const changesInspector = page.getByTestId("code-changes-inspector");
  await expect(changesInspector).toBeVisible();
  await expect(changesInspector).toContainText("2 changed files");
  await expect(changesInspector).toContainText("CodeWorkspaceScreen.tsx");
  await expect(changesInspector).toContainText("codeSessionReducer.ts");
  await expect(changesInspector).toContainText("+27");
  await expect(changesInspector).toContainText("-4");
  await expect(changesInspector).toContainText(
    'import { CodeChangesPanel } from "./CodeChangesPanel";',
  );
  await expect(
    changesInspector.getByRole("button", {
      name: /stage|commit|push|pull request/i,
    }),
  ).toHaveCount(0);
  await expect(
    changesInspector.getByTestId("project-diff-add-comment"),
  ).toHaveCount(0);

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
      model: "gpt-5.2-codex",
      effort: "medium",
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

test("resizes and remembers the SchoolX Code side panels", async ({ page }) => {
  await page.setViewportSize({ width: 1_600, height: 900 });
  await installMockBridge(page, { schoolxCodeWorkspace: true });
  await openBoundThread(page);

  const tasks = page.getByTestId("code-thread-sidebar");
  const tasksResize = page.getByTestId("code-tasks-resize-handle");
  const initialTasksBox = await tasks.boundingBox();
  const tasksResizeBox = await tasksResize.boundingBox();
  expect(initialTasksBox).not.toBeNull();
  expect(tasksResizeBox).not.toBeNull();

  await page.mouse.move(
    (tasksResizeBox?.x ?? 0) + (tasksResizeBox?.width ?? 0) / 2,
    (tasksResizeBox?.y ?? 0) + (tasksResizeBox?.height ?? 0) / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    (tasksResizeBox?.x ?? 0) + (tasksResizeBox?.width ?? 0) / 2 + 80,
    (tasksResizeBox?.y ?? 0) + (tasksResizeBox?.height ?? 0) / 2,
  );
  await page.mouse.up();

  await expect
    .poll(async () => (await tasks.boundingBox())?.width ?? 0)
    .toBeGreaterThan((initialTasksBox?.width ?? 0) + 70);

  const changesFrame = page.getByTestId("code-changes-panel-frame");
  const changesResize = page.getByTestId("code-changes-resize-handle");
  const initialChangesBox = await changesFrame.boundingBox();
  expect(initialChangesBox).not.toBeNull();
  await changesResize.focus();
  await page.keyboard.press("ArrowLeft");
  await expect
    .poll(async () => (await changesFrame.boundingBox())?.width ?? 0)
    .toBeGreaterThan((initialChangesBox?.width ?? 0) + 12);

  await expect
    .poll(() =>
      page.evaluate(() => {
        const raw = window.localStorage.getItem(
          "buzz.desktop.schoolx-code-panel-widths",
        );
        return raw ? JSON.parse(raw) : null;
      }),
    )
    .toMatchObject({ tasks: 320, changes: 560 });

  await page.reload();
  await expect(codeRuntimeReadyLabel(page)).toBeVisible({ timeout: 15_000 });
  await expect(tasks).toBeVisible();
  await expect(changesFrame).toBeVisible();
  await expect
    .poll(async () => ({
      changes: Math.round((await changesFrame.boundingBox())?.width ?? 0),
      tasks: Math.round((await tasks.boundingBox())?.width ?? 0),
    }))
    .toEqual({ tasks: 320, changes: 560 });
});

test("locks task refresh while a new Codex thread opens without relisting it", async ({
  page,
}) => {
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeWorktreePrepareDelayMs: 2_000,
    schoolxCodeThreadStartDelayMs: 2_000,
  });
  await openCodeWorkspace(page);

  const listCount = (await commandPayloads(page, "code_threads_list")).length;
  await page.getByRole("button", { name: "New task", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "New Code task" });
  const create = dialog.getByTestId("code-new-task-submit");
  await create.evaluate((button) => {
    button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  await waitForCommand(page, "code_worktree_prepare");
  const creationLock = await page.evaluate(() => {
    const submit = document.querySelector<HTMLButtonElement>(
      '[data-testid="code-new-task-submit"]',
    );
    const refresh = document.querySelector<HTMLButtonElement>(
      'button[aria-label="Refresh Code tasks"]',
    );
    return {
      refreshDisabled: refresh?.disabled,
      submitBusy: submit?.getAttribute("aria-busy"),
      submitDisabled: submit?.disabled,
    };
  });
  expect(creationLock).toEqual({
    refreshDisabled: true,
    submitBusy: "true",
    submitDisabled: true,
  });
  expect(await commandPayloads(page, "code_worktree_prepare")).toHaveLength(1);
  const refresh = page.getByRole("button", { name: "Refresh Code tasks" });
  await waitForCommand(page, "code_thread_start");
  const startLock = await page.evaluate(() => {
    const submit = document.querySelector<HTMLButtonElement>(
      '[data-testid="code-new-task-submit"]',
    );
    const refresh = document.querySelector<HTMLButtonElement>(
      'button[aria-label="Refresh Code tasks"]',
    );
    return {
      refreshDisabled: refresh?.disabled,
      submitBusy: submit?.getAttribute("aria-busy"),
      submitDisabled: submit?.disabled,
    };
  });
  expect(startLock).toEqual({
    refreshDisabled: true,
    submitBusy: "true",
    submitDisabled: true,
  });
  expect(await commandPayloads(page, "code_thread_start")).toHaveLength(1);
  expect((await commandPayloads(page, "code_threads_list")).length).toBe(
    listCount,
  );

  await expect(
    page.getByTestId(`code-thread-${CREATED_THREAD_ID}`),
  ).toHaveAttribute("aria-current", "page");
  await expect(
    page.getByRole("textbox", { name: "Message Code task" }),
  ).toBeEnabled();
  await expect(refresh).toBeEnabled();
  expect((await commandPayloads(page, "code_threads_list")).length).toBe(
    listCount,
  );
});

test("recovers a failed Code task list through its explicit Refresh action", async ({
  page,
}) => {
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeThreadListErrors: [
      "Mock Code task list unavailable",
      "Mock Code task list unavailable",
      null,
    ],
  });
  await openCodeWorkspace(page);
  await waitForCommandCount(page, "code_threads_list", 2);

  await page.evaluate((threadId) => {
    const [path, queryString = ""] = window.location.hash.slice(1).split("?");
    const search = new URLSearchParams(queryString);
    search.set("threadId", threadId);
    window.location.hash = `${path}?${search.toString()}`;
  }, THREAD_ID);

  const listError = page
    .getByRole("alert")
    .filter({ hasText: "Mock Code task list unavailable" });
  await expect(listError).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`threadId=${THREAD_ID}`));
  const refresh = page.getByRole("button", { name: "Refresh Code tasks" });
  await expect(refresh).toBeEnabled();
  await expect(
    page.getByRole("button", { name: "New task", exact: true }),
  ).toBeDisabled();

  await refresh.click();
  await waitForCommandCount(page, "code_threads_list", 3);
  await expect(page.getByTestId(`code-thread-${THREAD_ID}`)).toHaveAttribute(
    "aria-current",
    "page",
  );
  await waitForCommand(page, "code_thread_resume");
  await expect(page.getByTestId("code-timeline")).toContainText(
    "Historical Code response from fixture.",
  );
  await expect(listError).toHaveCount(0);
  await expect(refresh).toBeEnabled();
  expect((await commandPayloads(page, "code_threads_list")).length).toBe(3);
});

test("stages one managed-worktree file only after receipt, refresh, and acknowledgement", async ({
  page,
}) => {
  const receipt: CodeGitIndexMutationReceipt = {
    operationId: "e".repeat(64),
    operation: "stage",
    scope: SCOPE,
    threadId: CREATED_THREAD_ID,
    requestGeneration: 0,
    beforeSnapshotId: "c".repeat(64),
    fileId: "a".repeat(64),
    disposition: "staged",
  };
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeGitStatuses: [
      gitReadyStatus({ staged: false, statusRevision: 1, writeGeneration: 0 }),
      gitReadyStatus({
        blockingReceipt: receipt,
        staged: true,
        statusRevision: 2,
        writeGeneration: 1,
      }),
      gitReadyStatus({ staged: true, statusRevision: 3, writeGeneration: 1 }),
    ],
  });
  await openCodeWorkspace(page);
  await createManagedTask(page);

  const changesInspector = page.getByTestId("code-changes-inspector");
  await waitForCommand(page, "code_thread_git_status");
  await expect(
    changesInspector.getByRole("button", {
      name: "Stage desktop/src/features/code/ui/CodeChangesPanel.tsx",
    }),
  ).toBeEnabled();
  await changesInspector
    .getByRole("button", {
      name: "Stage desktop/src/features/code/ui/CodeChangesPanel.tsx",
    })
    .click();

  await waitForCommand(page, "code_thread_git_acknowledge");
  await expect(
    changesInspector.getByRole("button", {
      name: "Unstage desktop/src/features/code/ui/CodeChangesPanel.tsx",
    }),
  ).toBeEnabled();
  expect(await commandPayload(page, "code_thread_git_stage")).toEqual({
    input: {
      scope: SCOPE,
      threadId: CREATED_THREAD_ID,
      writeGeneration: 0,
      snapshotId: "c".repeat(64),
      fileId: "a".repeat(64),
    },
  });
  expect(await commandPayload(page, "code_thread_git_acknowledge")).toEqual({
    input: {
      scope: SCOPE,
      threadId: CREATED_THREAD_ID,
      operationId: "e".repeat(64),
      writeGeneration: 1,
      snapshotId: "d".repeat(64),
    },
  });
});

test("reconciles a committed Git write after its response is lost", async ({
  page,
}) => {
  const message = "Preserve the commit message across response loss";
  const responseLoss = "Mock commit response was lost after native commit";
  const receipt: CodeGitCommitReceipt = {
    operationId: "f".repeat(64),
    operation: "commit",
    scope: SCOPE,
    threadId: CREATED_THREAD_ID,
    requestGeneration: 0,
    beforeSnapshotId: "d".repeat(64),
    previousHead: "1".repeat(40),
    commit: "2".repeat(40),
    tree: "3".repeat(40),
    disposition: "committed",
  };
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeGitCommitResponseLosses: [responseLoss],
    schoolxCodeGitStatuses: [
      gitReadyStatus({ staged: true, statusRevision: 1, writeGeneration: 0 }),
      gitReadyStatus({
        blockingReceipt: receipt,
        clean: true,
        staged: false,
        statusRevision: 2,
        writeGeneration: 1,
      }),
      gitReadyStatus({
        clean: true,
        staged: false,
        statusRevision: 3,
        writeGeneration: 1,
      }),
    ],
  });
  await openCodeWorkspace(page);
  await createManagedTask(page);

  const changesInspector = page.getByTestId("code-changes-inspector");
  await waitForCommand(page, "code_thread_git_status");
  await changesInspector
    .getByRole("button", { name: "Commit staged changes" })
    .click();
  const dialog = page.getByRole("dialog", { name: "Commit staged changes" });
  const commitMessage = dialog.getByRole("textbox", {
    name: "Commit message",
  });
  await commitMessage.fill(message);
  await dialog.getByRole("button", { name: "Commit staged changes" }).click();

  await waitForCommand(page, "code_thread_git_commit");
  await expect(dialog).toBeVisible();
  await expect(commitMessage).toBeEnabled();
  await expect(commitMessage).toHaveValue(message);
  await expect(dialog).toContainText(responseLoss);
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toBeHidden();
  const composer = page.getByRole("textbox", { name: "Message Code task" });
  await expect(composer).toBeDisabled();
  await expect(page.locator("#code-composer-disabled-reason")).toHaveText(
    responseLoss,
  );

  await changesInspector
    .getByRole("button", { name: "Check operation status" })
    .click();
  await waitForCommand(page, "code_thread_git_acknowledge");
  await expect(composer).toBeEnabled();
  await expect(changesInspector).toContainText("No staged changes.");
  await expect(changesInspector).toContainText("No unstaged changes.");

  expect(await commandPayload(page, "code_thread_git_commit")).toEqual({
    input: {
      scope: SCOPE,
      threadId: CREATED_THREAD_ID,
      writeGeneration: 0,
      snapshotId: "d".repeat(64),
      message,
    },
  });
  expect(await commandPayload(page, "code_thread_git_reconcile")).toEqual({
    input: { scope: SCOPE, threadId: CREATED_THREAD_ID },
  });
  expect(await commandPayload(page, "code_thread_git_acknowledge")).toEqual({
    input: {
      scope: SCOPE,
      threadId: CREATED_THREAD_ID,
      operationId: receipt.operationId,
      writeGeneration: 1,
      snapshotId: "9".repeat(64),
    },
  });
});

test("renders explicit partial Changes completeness, file status, and binary state", async ({
  page,
}) => {
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeChanges: [PARTIAL_THREAD_CHANGES],
  });
  await openBoundThread(page);

  await waitForCommandCount(page, "code_thread_changes", 1);
  const changesInspector = page.getByTestId("code-changes-inspector");
  const completenessWarning = changesInspector.getByTestId(
    "code-changes-completeness-warning",
  );
  await expect(completenessWarning).toContainText(
    "Showing 3 of 5 changed files. Review the local checkout for the complete file list.",
  );
  await expect(completenessWarning).toContainText(
    "1 file patch truncated. Review the local checkout for the complete diff.",
  );
  await expect(changesInspector).toContainText("+4");
  await expect(changesInspector).toContainText("-3");
  await expect(
    changesInspector.getByLabel("Modified file status"),
  ).toBeVisible();
  await expect(
    changesInspector.getByLabel("Deleted file status"),
  ).toBeVisible();
  await expect(
    changesInspector.getByLabel("Untracked file status"),
  ).toBeVisible();

  await changesInspector
    .getByRole("button", { name: /code-workspace\.bin/ })
    .click();
  await expect(changesInspector).toContainText("Binary");
  await expect(changesInspector).toContainText(
    "Binary file preview is not available.",
  );
});

test("keeps Changes refresh gated until initial activity synchronization completes", async ({
  page,
}) => {
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeBlockInitialEventReplay: true,
  });
  await openBoundThread(page);

  const changesInspector = page.getByTestId("code-changes-inspector");
  const refreshChanges = changesInspector.getByRole("button", {
    name: "Refresh changed files",
  });
  await expect(
    changesInspector.getByTestId("code-changes-sync-pending"),
  ).toHaveText("Waiting for Code activity synchronization…");
  await expect(refreshChanges).toBeDisabled();
  expect(await commandPayloads(page, "code_thread_changes")).toEqual([]);

  await page.evaluate(() => {
    const releaseReplay = window.__BUZZ_E2E_RELEASE_CODE_EVENT_REPLAY__;
    if (!releaseReplay) {
      throw new Error("Code event replay release seam is unavailable");
    }
    releaseReplay();
  });

  await waitForCommandCount(page, "code_thread_changes", 1);
  await expect(
    changesInspector.getByTestId("code-changes-sync-pending"),
  ).toBeHidden();
  await expect(refreshChanges).toBeEnabled();
  await waitForTwoAnimationFrames(page);
  expect(await commandPayloads(page, "code_thread_changes")).toEqual([
    { input: { scope: SCOPE, threadId: THREAD_ID } },
  ]);
});

test("invalidates stale Changes exactly once after same-generation truncated replay recovery", async ({
  page,
}) => {
  const truncatedCheckpoint: CodeEventCheckpoint = {
    runtimeGeneration: 7,
    sequenceWatermark: 0,
    activeTurns: [],
    pendingApprovals: [],
  };
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeEventBacklogs: [
      eventBacklog([]),
      eventBacklog([], true, 7, truncatedCheckpoint),
    ],
    schoolxCodeChanges: [STALE_THREAD_CHANGES, FRESH_THREAD_CHANGES],
  });
  await openBoundThread(page);

  const changesInspector = page.getByTestId("code-changes-inspector");
  const expectedChangesPayload = {
    input: { scope: SCOPE, threadId: THREAD_ID },
  };
  const expectedResumePayload = {
    input: { scope: SCOPE, threadId: THREAD_ID, model: null },
  };
  await waitForCommandCount(page, "code_thread_changes", 1);
  await expect(changesInspector).toContainText("staleSnapshot.ts");
  expect(await commandPayloads(page, "code_thread_changes")).toEqual([
    expectedChangesPayload,
  ]);

  await page.evaluate(
    async ({ scope, threadId }) => {
      const emitEvent = window.__BUZZ_E2E_EMIT_CODE_WORKSPACE_EVENT__;
      if (!emitEvent) {
        throw new Error("Code workspace event seam is unavailable");
      }
      await emitEvent({
        scope,
        runtimeGeneration: 7,
        sequence: -1,
        threadId,
        turnId: null,
        itemId: null,
        kind: "warning",
        payload: { message: "Force a full replay retry." },
      } as CodeWorkspaceEvent);
    },
    { scope: SCOPE, threadId: THREAD_ID },
  );

  const syncActivity = page.getByRole("button", { name: "Sync activity" });
  await expect(syncActivity).toBeVisible();
  await expect(
    changesInspector.getByTestId("code-changes-sync-pending"),
  ).toBeVisible();
  await expect(
    changesInspector.getByRole("button", { name: "Refresh changed files" }),
  ).toBeDisabled();
  expect(await commandPayloads(page, "code_thread_changes")).toEqual([
    expectedChangesPayload,
  ]);

  await syncActivity.click();
  await waitForCommandCount(page, "code_runtime_events", 2);
  expect(await commandPayloads(page, "code_runtime_events")).toEqual([
    { scope: SCOPE, runtimeGeneration: 7, afterSequence: 0 },
    { scope: SCOPE, runtimeGeneration: 7, afterSequence: 0 },
  ]);
  await waitForCommandCount(page, "code_thread_resume", 2);
  expect(await commandPayloads(page, "code_thread_resume")).toEqual([
    expectedResumePayload,
    expectedResumePayload,
  ]);
  await waitForCommandCount(page, "code_thread_changes", 2);
  await expect(changesInspector).toContainText("freshSnapshot.ts");
  await expect(changesInspector).not.toContainText("staleSnapshot.ts");
  await waitForTwoAnimationFrames(page);
  expect(await commandPayloads(page, "code_thread_changes")).toEqual([
    expectedChangesPayload,
    expectedChangesPayload,
  ]);
});

test("creates and selects a managed-worktree task without resuming its start result", async ({
  page,
}) => {
  await installMockBridge(page, { schoolxCodeWorkspace: true });
  await openBoundThread(page);

  await createManagedTask(page);
});

test("creates a local-checkout task only after keyboard selection and native state review", async ({
  page,
}) => {
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeLocalCheckoutState: {
      branch: "feature/local-checkout-ui",
      dirty: true,
    },
  });
  await openBoundThread(page);

  const newTask = page.getByRole("button", { name: "New task", exact: true });
  await newTask.focus();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog", { name: "New Code task" });
  const managedMode = dialog.getByRole("radio", {
    name: /Managed worktree/,
  });
  const localMode = dialog.getByRole("radio", { name: /Local checkout/ });
  await expect(managedMode).toBeChecked();
  await expect(localMode).not.toBeChecked();
  await expect(managedMode).toBeFocused();

  await page.keyboard.press("ArrowDown");
  await expect(localMode).toBeChecked();
  await expect(localMode).toBeFocused();
  await expect(dialog.getByTestId("code-local-checkout-warning")).toContainText(
    "Changes from this task apply directly to your existing checkout.",
  );
  expect(await commandPayloads(page, "code_worktree_prepare")).toEqual([]);

  const review = dialog.getByRole("button", {
    name: "Review local checkout",
  });
  await review.focus();
  await page.keyboard.press("Enter");
  await waitForCommand(page, "code_worktree_prepare");
  expect(await commandPayload(page, "code_worktree_prepare")).toEqual({
    input: {
      scope: SCOPE,
      repositoryRoot: "/mock/buzz",
      baseRef: "main",
      executionMode: "local",
    },
  });
  await expect(dialog.getByTestId("code-local-checkout-branch")).toHaveText(
    "feature/local-checkout-ui",
  );
  await expect(dialog.getByTestId("code-local-checkout-dirty")).toHaveText(
    "Uncommitted changes present",
  );
  expect(await commandPayloads(page, "code_worktree_status")).toEqual([]);
  expect(await commandPayloads(page, "code_thread_start")).toEqual([]);

  const confirm = dialog.getByRole("button", {
    name: "Create task in local checkout",
  });
  await confirm.focus();
  await page.keyboard.press("Enter");
  await waitForCommand(page, "code_worktree_status");
  expect(await commandPayload(page, "code_worktree_status")).toEqual({
    descriptor: {
      executionMode: "local",
      repositoryIdentity: SCOPE.repositoryIdentity,
      executionRoot: "/mock/buzz",
      baseRef: "main",
      worktreeId: null,
    },
  });
  await waitForCommand(page, "code_thread_start");
  expect(await commandPayload(page, "code_thread_start")).toEqual({
    input: {
      scope: SCOPE,
      preparationId: CREATED_PREPARATION_ID,
      model: "gpt-5.2-codex",
    },
  });
  await expect(page).toHaveURL(new RegExp(`threadId=${CREATED_THREAD_ID}`));
  const createdThread = page.getByTestId(`code-thread-${CREATED_THREAD_ID}`);
  await expect(createdThread).toContainText("Local checkout");
  await expect(
    page.getByRole("heading", { name: "Local checkout task", exact: true }),
  ).toBeVisible();

  await newTask.click();
  const resetDialog = page.getByRole("dialog", { name: "New Code task" });
  await expect(
    resetDialog.getByRole("radio", { name: /Managed worktree/ }),
  ).toBeChecked();
  await expect(
    resetDialog.getByRole("radio", { name: /Local checkout/ }),
  ).not.toBeChecked();
  await page.keyboard.press("Escape");
  await expect(resetDialog).toHaveCount(0);
  await expect(newTask).toBeFocused();
});

test("keeps a deferred Local preparation behind native state review", async ({
  page,
}) => {
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeLocalCheckoutState: { branch: "feature/deferred", dirty: true },
  });
  await openBoundThread(page);

  await page.getByRole("button", { name: "New task", exact: true }).click();
  let dialog = page.getByRole("dialog", { name: "New Code task" });
  await dialog.getByRole("radio", { name: /Local checkout/ }).click();
  await dialog.getByRole("button", { name: "Review local checkout" }).click();
  await expect(dialog.getByTestId("code-local-checkout-branch")).toHaveText(
    "feature/deferred",
  );
  await dialog.getByRole("button", { name: "Continue later" }).click();

  const preparation = page.getByTestId(
    `code-preparation-${CREATED_PREPARATION_ID}`,
  );
  await expect(preparation).toContainText("Prepared task");
  await preparation.getByRole("button", { name: "Start task" }).click();
  dialog = page.getByRole("dialog", { name: "New Code task" });
  const localMode = dialog.getByRole("radio", { name: /Local checkout/ });
  await expect(localMode).toBeChecked();
  await expect(localMode).toBeDisabled();
  await expect(dialog.getByTestId("code-local-checkout-branch")).toHaveCount(0);

  await dialog.getByRole("button", { name: "Review local checkout" }).click();
  await waitForCommand(page, "code_worktree_status");
  expect(await commandPayload(page, "code_worktree_status")).toEqual({
    descriptor: {
      executionMode: "local",
      repositoryIdentity: SCOPE.repositoryIdentity,
      executionRoot: "/mock/buzz",
      baseRef: "main",
      worktreeId: null,
    },
  });
  await expect(dialog.getByTestId("code-local-checkout-branch")).toHaveText(
    "feature/deferred",
  );
  await expect(dialog.getByTestId("code-local-checkout-dirty")).toHaveText(
    "Uncommitted changes present",
  );
  expect(await commandPayloads(page, "code_thread_start")).toEqual([]);

  await dialog
    .getByRole("button", { name: "Create task in local checkout" })
    .click();
  await waitForCommandCount(page, "code_worktree_status", 2);
  await waitForCommand(page, "code_thread_start");
  expect(await commandPayload(page, "code_thread_start")).toEqual({
    input: {
      scope: SCOPE,
      preparationId: CREATED_PREPARATION_ID,
      model: "gpt-5.2-codex",
    },
  });
});

test("shows native preparation, revalidation, and uncertain-start failures beside the Local action", async ({
  page,
}) => {
  const preparationError =
    "Native preparation refused the checkout after its dirty-state inspection.";
  const revalidationError =
    "Native revalidation found that the local checkout authority drifted.";
  const uncertainStartError =
    "Native could not determine whether the Local task started.";
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeWorktreePrepareErrors: [preparationError, null],
    schoolxCodeWorktreeStatusErrors: [revalidationError, null],
    schoolxCodeThreadStartErrors: [
      { code: "threadStartUncertain", message: uncertainStartError },
    ],
  });
  await openBoundThread(page);

  await page.getByRole("button", { name: "New task", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "New Code task" });
  await dialog.getByRole("radio", { name: /Local checkout/ }).click();
  await dialog.getByRole("button", { name: "Review local checkout" }).click();
  await expect(dialog.getByTestId("code-task-creation-error")).toContainText(
    preparationError,
  );
  expect(await commandPayloads(page, "code_worktree_prepare")).toHaveLength(1);
  expect(await commandPayloads(page, "code_thread_start")).toEqual([]);

  await dialog.getByRole("button", { name: "Review local checkout" }).click();
  await waitForCommandCount(page, "code_worktree_prepare", 2);
  await expect(dialog.getByTestId("code-local-checkout-branch")).toHaveText(
    "main",
  );
  await dialog
    .getByRole("button", { name: "Create task in local checkout" })
    .click();
  await expect(dialog.getByTestId("code-task-creation-error")).toContainText(
    revalidationError,
  );
  expect(await commandPayloads(page, "code_worktree_status")).toHaveLength(1);
  expect(await commandPayloads(page, "code_thread_start")).toEqual([]);

  await dialog
    .getByRole("button", { name: "Create task in local checkout" })
    .click();
  await waitForCommandCount(page, "code_worktree_status", 2);
  await waitForCommand(page, "code_thread_start");
  await expect(dialog.getByTestId("code-task-creation-error")).toContainText(
    uncertainStartError,
  );
  await expect(dialog.getByTestId("code-task-creation-error")).toContainText(
    "continue from Unfinished",
  );
  await expect(dialog.getByTestId("code-new-task-submit")).toBeDisabled();
  expect(await commandPayloads(page, "code_thread_start")).toHaveLength(1);

  await dialog.getByRole("button", { name: "Continue later" }).click();
  const recovery = page.getByTestId(
    `code-preparation-${CREATED_PREPARATION_ID}`,
  );
  await expect(recovery).toContainText("Needs recovery");
  await recovery.getByRole("button", { name: "Recover task" }).click();
  await waitForCommand(page, "code_thread_binding_recover");
  expect(await commandPayload(page, "code_thread_binding_recover")).toEqual({
    input: {
      scope: SCOPE,
      preparationId: CREATED_PREPARATION_ID,
      model: null,
    },
  });
  expect(await commandPayloads(page, "code_thread_start")).toHaveLength(1);
});

test("requires another Local confirmation after native branch or dirty-state drift", async ({
  page,
}) => {
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeLocalCheckoutState: { branch: "main", dirty: false },
    schoolxCodeWorktreeStatusStates: [
      { branch: "feature/drifted", dirty: true },
    ],
  });
  await openBoundThread(page);

  await page.getByRole("button", { name: "New task", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "New Code task" });
  await dialog.getByRole("radio", { name: /Local checkout/ }).click();
  await dialog.getByRole("button", { name: "Review local checkout" }).click();
  await expect(dialog.getByTestId("code-local-checkout-branch")).toHaveText(
    "main",
  );
  await expect(dialog.getByTestId("code-local-checkout-dirty")).toHaveText(
    "Clean",
  );

  await dialog
    .getByRole("button", { name: "Create task in local checkout" })
    .click();
  await waitForCommand(page, "code_worktree_status");
  await expect(dialog.getByTestId("code-local-checkout-drift")).toBeVisible();
  await expect(dialog.getByTestId("code-local-checkout-branch")).toHaveText(
    "feature/drifted",
  );
  await expect(dialog.getByTestId("code-local-checkout-dirty")).toHaveText(
    "Uncommitted changes present",
  );
  expect(await commandPayloads(page, "code_thread_start")).toEqual([]);

  await dialog
    .getByRole("button", { name: "Create task in local checkout" })
    .click();
  await waitForCommandCount(page, "code_worktree_status", 2);
  await waitForCommand(page, "code_thread_start");
  expect(await commandPayloads(page, "code_worktree_status")).toHaveLength(2);
  expect(await commandPayloads(page, "code_thread_start")).toHaveLength(1);
});

test("persists model and reasoning selection into exact start and turn payloads", async ({
  page,
}) => {
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeModelSelectionErrors: ["Mock selection write failed", null],
  });
  await openBoundThread(page);

  await waitForCommand(page, "code_models_list");
  expect(await commandPayload(page, "code_models_list")).toEqual({});

  const modelSelector = page.getByTestId("code-model-selector");
  const reasoningSelector = page.getByTestId("code-reasoning-selector");
  await expect(modelSelector).toHaveAccessibleName("Model: GPT-5.2 Codex");
  await expect(reasoningSelector).toHaveAccessibleName(
    "Reasoning effort: Medium",
  );

  await modelSelector.focus();
  await page.keyboard.press("Enter");
  const miniOption = page
    .getByRole("menuitemradio")
    .filter({ hasText: "Codex Mini" });
  await miniOption.focus();
  await page.keyboard.press("Enter");
  await waitForCommandCount(page, "code_model_selection_set", 1);
  const selectorError = page.getByTestId("code-model-selector-error");
  await expect(selectorError).toContainText("Model selection wasn’t saved");
  await expect(modelSelector).toHaveAccessibleName("Model: GPT-5.2 Codex");
  await expect(modelSelector).toBeFocused();

  const retrySelection = selectorError.getByRole("button", {
    name: "Retry",
  });
  await retrySelection.focus();
  await page.keyboard.press("Enter");
  await waitForCommandCount(page, "code_model_selection_set", 2);
  await expect(selectorError).toHaveCount(0);
  expect(await commandPayloads(page, "code_model_selection_set")).toEqual([
    {
      input: {
        model: "codex-mini-latest",
        reasoningEffort: "medium",
      },
    },
    {
      input: {
        model: "codex-mini-latest",
        reasoningEffort: "medium",
      },
    },
  ]);
  await expect(modelSelector).toHaveAccessibleName("Model: Codex Mini");

  await reasoningSelector.focus();
  await page.keyboard.press("Enter");
  const lowOption = page.getByRole("menuitemradio").filter({ hasText: /^Low/ });
  await lowOption.focus();
  await page.keyboard.press("Enter");
  await waitForCommandCount(page, "code_model_selection_set", 3);
  await expect(reasoningSelector).toBeFocused();
  expect(await commandPayloads(page, "code_model_selection_set")).toEqual([
    {
      input: {
        model: "codex-mini-latest",
        reasoningEffort: "medium",
      },
    },
    {
      input: {
        model: "codex-mini-latest",
        reasoningEffort: "medium",
      },
    },
    {
      input: {
        model: "codex-mini-latest",
        reasoningEffort: "low",
      },
    },
  ]);
  await expect(reasoningSelector).toHaveAccessibleName("Reasoning effort: Low");

  await createManagedTask(page, "codex-mini-latest");
  await expect(modelSelector).toHaveAccessibleName("Model: Codex Mini");
  await expect(reasoningSelector).toHaveAccessibleName("Reasoning effort: Low");

  const composer = page.getByRole("textbox", { name: "Message Code task" });
  await composer.fill(SELECTOR_PROMPT);
  await page.getByRole("button", { name: "Send prompt" }).click();
  await waitForCommand(page, "code_turn_start");
  expect(await commandPayload(page, "code_turn_start")).toEqual({
    input: {
      scope: SCOPE,
      threadId: CREATED_THREAD_ID,
      prompt: SELECTOR_PROMPT,
      model: "codex-mini-latest",
      effort: "low",
    },
  });
  await expect(modelSelector).toHaveAttribute("aria-disabled", "true");
  await expect(reasoningSelector).toHaveAttribute("aria-disabled", "true");
  await modelSelector.focus();
  await expect(modelSelector).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.getByText("Model", { exact: true })).toHaveCount(0);
  expect(await commandPayloads(page, "code_thread_resume")).toEqual([
    {
      input: {
        scope: SCOPE,
        threadId: THREAD_ID,
        model: null,
      },
    },
  ]);
});

test("falls back to null model inputs when the catalog is unavailable", async ({
  page,
}) => {
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeModelCatalogErrors: [
      "Mock model catalog unavailable",
      "Mock model catalog still unavailable",
    ],
  });
  await openBoundThread(page);

  await waitForCommand(page, "code_models_list");
  const selectorError = page.getByTestId("code-model-selector-error");
  await expect(selectorError).toContainText("Model options are unavailable");
  await expect(page.getByTestId("code-model-selector")).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  await expect(page.getByTestId("code-reasoning-selector")).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  const retryCatalog = selectorError.getByRole("button", {
    name: "Retry",
  });
  await retryCatalog.focus();
  await page.keyboard.press("Enter");
  await waitForCommandCount(page, "code_models_list", 2);
  expect(await commandPayloads(page, "code_models_list")).toEqual([{}, {}]);
  await expect(selectorError).toContainText("Model options are unavailable");

  await createManagedTask(page, null);
  const composer = page.getByRole("textbox", { name: "Message Code task" });
  await composer.fill(CATALOG_FALLBACK_PROMPT);
  await page.getByRole("button", { name: "Send prompt" }).click();
  await waitForCommand(page, "code_turn_start");
  expect(await commandPayload(page, "code_turn_start")).toEqual({
    input: {
      scope: SCOPE,
      threadId: CREATED_THREAD_ID,
      prompt: CATALOG_FALLBACK_PROMPT,
      model: null,
      effort: null,
    },
  });
});

test("lists managed-root removal eligibility and recovers an inventory read failure", async ({
  page,
}) => {
  await installMockBridge(page, { schoolxCodeWorkspace: true });
  await openCodeWorkspace(page);
  await waitForCommand(page, "code_worktrees_list");

  const exactPayload = { input: { scope: SCOPE } };
  expect(await commandPayloads(page, "code_worktrees_list")).toEqual([
    exactPayload,
  ]);

  const inventory = page.getByTestId("code-worktree-inventory");
  await inventory.scrollIntoViewIfNeeded();
  await expect(inventory).toBeVisible();

  const rows = inventory.locator('li[data-testid^="code-worktree-"]');
  await expect(rows).toHaveCount(6);

  const active = page.getByTestId(
    "code-worktree-11111111-1111-4111-8111-111111111111",
  );
  await expect(active).toContainText("Task · active");
  await expect(active).toContainText("Active task");
  await expect(active).toContainText("Preserved");

  const archived = page.getByTestId(
    "code-worktree-22222222-2222-4222-8222-222222222222",
  );
  await expect(archived).toContainText("Task · archived");
  await expect(archived).toContainText("Merge proof unavailable");
  await expect(archived).toContainText("Preserved");

  const unavailable = page.getByTestId(
    "code-worktree-33333333-3333-4333-8333-333333333333",
  );
  await expect(unavailable).toContainText("Task · archived");
  await expect(unavailable).toContainText(
    "Mock managed worktree root is unavailable",
  );
  await expect(unavailable).toContainText("Worktree unavailable");
  await expect(unavailable).toContainText("Merge proof unavailable");
  await expect(active).toBeVisible();

  const removable = page.getByTestId(
    "code-worktree-66666666-6666-4666-8666-666666666666",
  );
  await expect(removable).toContainText("Task · archived");
  await expect(removable).toContainText("Ready to remove");
  await expect(removable).not.toContainText("Merge proof unavailable");
  await expect(
    removable.getByRole("button", {
      name: "Remove worktree for task thread-inventory-removable",
    }),
  ).toBeEnabled();

  const startPreparation = page.getByTestId(
    "code-worktree-44444444-4444-4444-8444-444444444444",
  );
  await expect(startPreparation).toContainText("Task preparation · prepared");
  await expect(startPreparation).toContainText("Unfinished task preparation");

  const forkPreparation = page.getByTestId(
    "code-worktree-55555555-5555-4555-8555-555555555555",
  );
  await expect(forkPreparation).toContainText("Fork preparation · starting");
  await expect(forkPreparation).toContainText("Unfinished task preparation");

  await expect(inventory.getByText("Preserved", { exact: true })).toHaveCount(
    5,
  );
  await expect(
    inventory.getByText("Local checkout", { exact: true }),
  ).toHaveCount(0);
  await expect(
    inventory.getByRole("button", {
      name: /remove|delete|destroy|cleanup|clean|prune|purge|discard/i,
    }),
  ).toHaveCount(1);

  const rowsBeforeReadFailure = await rows.allTextContents();
  await page.evaluate(() => {
    const setInventoryError =
      window.__BUZZ_E2E_SET_CODE_WORKTREE_INVENTORY_ERROR__;
    if (!setInventoryError) {
      throw new Error("Managed-worktree inventory error seam is unavailable");
    }
    setInventoryError(
      "Mock inventory read failed without changing managed roots",
    );
  });
  await inventory
    .getByRole("button", { name: "Refresh managed worktrees" })
    .click();
  const inventoryError = inventory.getByRole("alert");
  await expect(inventoryError).toContainText(
    "Mock inventory read failed without changing managed roots",
  );

  await page.evaluate(() => {
    const setInventoryError =
      window.__BUZZ_E2E_SET_CODE_WORKTREE_INVENTORY_ERROR__;
    if (!setInventoryError) {
      throw new Error("Managed-worktree inventory error seam is unavailable");
    }
    setInventoryError(null);
  });
  await inventoryError.getByRole("button", { name: "Retry inventory" }).click();

  await expect(inventoryError).toHaveCount(0);
  await expect(rows).toHaveCount(6);
  expect(await rows.allTextContents()).toEqual(rowsBeforeReadFailure);
  expect(await commandPayloads(page, "code_worktrees_list")).toEqual([
    exactPayload,
    exactPayload,
    exactPayload,
    exactPayload,
  ]);
  expect(
    await page.evaluate(() =>
      (window.__BUZZ_E2E_COMMANDS__ ?? []).filter((command) =>
        /^code_worktree.*(?:remove|delete|destroy|cleanup|clean|prune|purge|discard)/i.test(
          command,
        ),
      ),
    ),
  ).toEqual([]);
});

test("confirms exact worktree removal without optimistic row deletion and retries safely", async ({
  page,
}) => {
  await installMockBridge(page, {
    schoolxCodeBlockRemoval: true,
    schoolxCodeRemovalResponseLosses: [
      "Mock response was lost after native removal committed",
    ],
    schoolxCodeWorkspace: true,
  });
  await openCodeWorkspace(page);
  await waitForCommand(page, "code_worktrees_list");

  const inventory = page.getByTestId("code-worktree-inventory");
  await inventory.scrollIntoViewIfNeeded();
  const exactPayload = {
    input: {
      scope: SCOPE,
      threadId: "thread-inventory-removable",
    },
  };
  const preservedWorktreeIds = [
    "11111111-1111-4111-8111-111111111111",
    "22222222-2222-4222-8222-222222222222",
    "33333333-3333-4333-8333-333333333333",
    "44444444-4444-4444-8444-444444444444",
    "55555555-5555-4555-8555-555555555555",
  ];
  const removable = page.getByTestId(
    "code-worktree-66666666-6666-4666-8666-666666666666",
  );
  const removableThread = page.getByTestId(
    "code-thread-thread-inventory-removable",
  );
  await expect(removableThread).toBeVisible();
  const removeButton = removable.getByRole("button", {
    name: "Remove worktree for task thread-inventory-removable",
  });
  await removeButton.click();

  const dialog = page.getByTestId("code-worktree-remove-dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText("The Code task transcript is preserved");
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(removeButton).toBeFocused();
  expect(await commandPayloads(page, "code_worktree_remove")).toEqual([]);
  await expect(removable).toBeVisible();

  await removeButton.click();
  await dialog.getByRole("button", { name: "Remove worktree" }).click();
  await waitForCommandCount(page, "code_worktree_remove", 1);
  await expect(removable).toBeVisible();
  await expect(dialog).toContainText("Confirming…");

  const concurrentRetry = invokeMockCommandError(
    page,
    "code_worktree_remove",
    exactPayload,
  );
  await waitForCommandCount(page, "code_worktree_remove", 2);
  await expect(
    inventory.locator('li[data-testid^="code-worktree-"]'),
  ).toHaveCount(6);
  await page.evaluate(() => {
    const releaseRemoval = window.__BUZZ_E2E_RELEASE_CODE_WORKTREE_REMOVAL__;
    if (!releaseRemoval) {
      throw new Error("Managed-worktree removal release gate is unavailable");
    }
    releaseRemoval();
  });
  expect((await concurrentRetry)?.message).toContain(
    "Mock response was lost after native removal committed",
  );
  await expect(dialog.getByRole("alert")).toContainText(
    "Mock response was lost after native removal committed",
  );
  await expect(removable).toBeVisible();

  await dialog.getByRole("button", { name: "Close" }).click();
  await expect(dialog).toHaveCount(0);
  const recoverReceiptButton = removable.getByRole("button", {
    name: "Retry exact worktree removal for task thread-inventory-removable",
  });
  await expect(recoverReceiptButton).toBeFocused();
  await page.getByRole("button", { name: "Hide task sidebar" }).click();
  await expect(inventory).toHaveCount(0);
  await page.getByRole("button", { name: "Show task sidebar" }).click();
  await expect(inventory).toBeVisible();
  await expect(removable).toHaveCount(0);
  const reconciliation = page.getByTestId(
    "code-worktree-removal-reconciliation",
  );
  await expect(reconciliation).toContainText(
    "The removal outcome for task thread-inventory-removable is unknown",
  );
  const retryExactRemoval = reconciliation.getByRole("button", {
    name: "Review exact removal retry",
  });

  await page.evaluate(() => {
    const setInventoryError =
      window.__BUZZ_E2E_SET_CODE_WORKTREE_INVENTORY_ERROR__;
    if (!setInventoryError) {
      throw new Error("Managed-worktree inventory error seam is unavailable");
    }
    setInventoryError("Mock post-removal inventory refresh failed");
  });
  await retryExactRemoval.click();
  await expect(dialog).toContainText(
    "Retry exact worktree removal for task thread-inventory-removable?",
  );
  await dialog.getByRole("button", { name: "Retry exact removal" }).click();
  await waitForCommandCount(page, "code_worktree_remove", 3);
  await expect(dialog.getByRole("alert")).toContainText(
    "Worktree removal completed",
  );
  await expect(dialog.getByRole("alert")).toContainText(
    "Mock post-removal inventory refresh failed",
  );
  await dialog.getByRole("button", { name: "Close" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(reconciliation).toContainText("Worktree removal completed");
  await page.getByRole("button", { name: "Hide task sidebar" }).click();
  await expect(inventory).toHaveCount(0);
  await page.getByRole("button", { name: "Show task sidebar" }).click();
  await expect(inventory).toBeVisible();
  await expect(reconciliation).toContainText(
    "Worktree removal completed for task thread-inventory-removable",
  );
  const authoritativeRefresh = inventory.getByRole("button", {
    name: "Refresh authoritative task lists after worktree removal",
  });
  await expect(authoritativeRefresh).toBeVisible();
  const retryTaskListRefresh = reconciliation.getByRole("button", {
    name: "Retry task-list refresh",
  });
  await page.evaluate(() => {
    const setInventoryError =
      window.__BUZZ_E2E_SET_CODE_WORKTREE_INVENTORY_ERROR__;
    if (!setInventoryError) {
      throw new Error("Managed-worktree inventory error seam is unavailable");
    }
    setInventoryError(null);
  });
  await retryTaskListRefresh.click();
  await expect(removable).toHaveCount(0);
  await expect(dialog).toHaveCount(0);
  await expect
    .poll(
      async () => (await commandPayloads(page, "code_worktree_remove")).length,
    )
    .toBe(3);
  await expect(
    inventory.getByRole("button", { name: "Refresh managed worktrees" }),
  ).toBeFocused();
  await expect(
    inventory.locator('[role="status"][aria-live="polite"]'),
  ).toContainText(
    "Worktree removed for task thread-inventory-removable. Its Code task transcript was preserved.",
  );
  await expect(
    inventory.locator('li[data-testid^="code-worktree-"]'),
  ).toHaveCount(5);
  for (const worktreeId of preservedWorktreeIds) {
    await expect(page.getByTestId(`code-worktree-${worktreeId}`)).toBeVisible();
  }
  await expect(removableThread).toHaveCount(0);
  await expect(page.getByTestId(`code-thread-${THREAD_ID}`)).toBeVisible();
  for (const peerThreadId of [
    "thread-inventory-active",
    "thread-inventory-archived",
    "thread-inventory-unavailable",
  ]) {
    await expect(page.getByTestId(`code-thread-${peerThreadId}`)).toHaveCount(
      1,
    );
  }
  expect(await commandPayloads(page, "code_worktree_remove")).toEqual([
    exactPayload,
    exactPayload,
    exactPayload,
  ]);

  const firstRetry = await invokeMockCommand(
    page,
    "code_worktree_remove",
    exactPayload,
  );
  const secondRetry = await invokeMockCommand(
    page,
    "code_worktree_remove",
    exactPayload,
  );
  expect(secondRetry).toEqual(firstRetry);
  expect(firstRetry).toMatchObject({
    removalId: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
    scope: SCOPE,
    threadId: "thread-inventory-removable",
    worktreeId: "66666666-6666-4666-8666-666666666666",
    transcriptDisposition: "preserved",
    executionDisposition: "removed",
  });

  const forbiddenInputError = await invokeMockCommandError(
    page,
    "code_worktree_remove",
    {
      input: {
        ...exactPayload.input,
        executionRoot: "/caller/substitution",
      },
    },
  );
  expect(forbiddenInputError?.message).toContain(
    "Worktree removal crossed its native trust boundary",
  );

  const foreignThreadPage = await invokeMockCommand(page, "code_threads_list", {
    input: {
      scope: { ...SCOPE, projectDtag: "project-foreign" },
    },
  });
  expect(foreignThreadPage).toEqual({
    data: [],
    nextCursor: null,
    backwardsCursor: null,
  });
  const forbiddenThreadListInput = await invokeMockCommandError(
    page,
    "code_threads_list",
    {
      input: {
        scope: SCOPE,
        executionRoot: "/caller/substitution",
      },
    },
  );
  expect(forbiddenThreadListInput?.message).toContain(
    "Code task list crossed its native trust boundary",
  );
});

test("fails closed outside exact mocked Code preparations and bindings", async ({
  page,
}) => {
  await installMockBridge(page, { schoolxCodeWorkspace: true });
  await enterMockApp(page);
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ === "function",
  );

  const modelCatalog = await invokeMockCommand(page, "code_models_list", {});
  expect(modelCatalog).toMatchObject({
    runtimeGeneration: 7,
    models: expect.arrayContaining([
      expect.objectContaining({
        id: "codex-mini",
        model: "codex-mini-latest",
      }),
    ]),
    recentSelection: {
      model: "gpt-5.2-codex",
      reasoningEffort: "medium",
    },
  });
  expect(
    await invokeMockCommandError(page, "code_models_list", {
      input: {},
    }),
  ).toEqual({
    message: "Model catalog requires an exact no-argument envelope",
    payload: null,
  });
  expect(
    await invokeMockCommandError(page, "code_model_selection_set", {
      input: {
        model: "gpt-5.2-codex",
        reasoningEffort: "medium",
        hidden: true,
      },
    }),
  ).toEqual({
    message: "Model selection crossed its native trust boundary",
    payload: null,
  });
  expect(
    await invokeMockCommandError(page, "code_model_selection_set", {
      input: {
        model: "gpt-5.2-codex",
        reasoningEffort: "unsupported",
      },
    }),
  ).toEqual({
    message: "Model selection crossed its native trust boundary",
    payload: null,
  });

  expect(
    await invokeMockCommand(page, "code_thread_resume", {
      input: { scope: SCOPE, threadId: THREAD_ID, model: null },
    }),
  ).toMatchObject({
    binding: {
      ...SCOPE,
      codexThreadId: THREAD_ID,
      executionMode: "local",
      executionRoot: "/mock/buzz",
    },
    thread: { id: THREAD_ID, cwd: "/mock/buzz" },
  });

  expect(
    await invokeMockCommandError(page, "code_thread_resume", {
      input: {
        scope: SCOPE,
        threadId: "thread-e2e-wrong",
        model: null,
      },
    }),
  ).toEqual(THREAD_SCOPE_FAILURE);
  const mismatchedScopes = [
    { ...SCOPE, communityId: "other-community" },
    { ...SCOPE, projectDtag: "other-project" },
    { ...SCOPE, repositoryIdentity: "d5".repeat(32) },
  ];
  for (const scope of mismatchedScopes) {
    expect(
      await invokeMockCommandError(page, "code_thread_resume", {
        input: { scope, threadId: THREAD_ID, model: null },
      }),
    ).toEqual(THREAD_SCOPE_FAILURE);
  }

  expect(
    await invokeMockCommand(page, "code_worktree_prepare", {
      input: {
        scope: SCOPE,
        repositoryRoot: "/mock/buzz",
        baseRef: "main",
        executionMode: "worktree",
      },
    }),
  ).toMatchObject({
    preparationId: CREATED_PREPARATION_ID,
    scope: SCOPE,
  });

  expect(
    await invokeMockCommandError(page, "code_thread_start", {
      input: {
        scope: SCOPE,
        preparationId: "preparation-e2e-wrong",
        model: null,
      },
    }),
  ).toEqual(PREPARATION_SCOPE_FAILURE);
  for (const scope of mismatchedScopes) {
    expect(
      await invokeMockCommandError(page, "code_thread_start", {
        input: {
          scope,
          preparationId: CREATED_PREPARATION_ID,
          model: null,
        },
      }),
    ).toEqual(PREPARATION_SCOPE_FAILURE);
  }

  expect(
    await invokeMockCommand(page, "code_thread_start", {
      input: {
        scope: SCOPE,
        preparationId: CREATED_PREPARATION_ID,
        model: null,
      },
    }),
  ).toMatchObject({
    binding: {
      ...SCOPE,
      codexThreadId: CREATED_THREAD_ID,
      executionMode: "worktree",
      executionRoot: "/mock/buzz-worktrees/code-e2e-new",
    },
    thread: {
      id: CREATED_THREAD_ID,
      cwd: "/mock/buzz-worktrees/code-e2e-new",
    },
  });
  expect(
    await invokeMockCommandError(page, "code_thread_start", {
      input: {
        scope: SCOPE,
        preparationId: CREATED_PREPARATION_ID,
        model: null,
      },
    }),
  ).toEqual(PREPARATION_SCOPE_FAILURE);

  expect(
    await invokeMockCommand(page, "code_thread_resume", {
      input: { scope: SCOPE, threadId: CREATED_THREAD_ID, model: null },
    }),
  ).toMatchObject({
    binding: {
      ...SCOPE,
      codexThreadId: CREATED_THREAD_ID,
      executionMode: "worktree",
      executionRoot: "/mock/buzz-worktrees/code-e2e-new",
    },
    thread: {
      id: CREATED_THREAD_ID,
      cwd: "/mock/buzz-worktrees/code-e2e-new",
    },
  });
  for (const scope of mismatchedScopes) {
    expect(
      await invokeMockCommandError(page, "code_thread_resume", {
        input: { scope, threadId: CREATED_THREAD_ID, model: null },
      }),
    ).toEqual(THREAD_SCOPE_FAILURE);
  }
});

test("enables Stop only for a normalized active turn and interrupts its exact identity", async ({
  page,
}) => {
  await installMockBridge(page, { schoolxCodeWorkspace: true });
  await openBoundThread(page);

  const composer = page.getByRole("textbox", { name: "Message Code task" });
  await composer.fill(INTERRUPT_PROMPT);
  await page.getByRole("button", { name: "Send prompt" }).click();
  await waitForCommand(page, "code_turn_start");
  expect(await commandPayload(page, "code_turn_start")).toEqual({
    input: {
      scope: SCOPE,
      threadId: THREAD_ID,
      prompt: INTERRUPT_PROMPT,
      model: "gpt-5.2-codex",
      effort: "medium",
    },
  });

  await expect(
    page.getByRole("textbox", { name: "Steer active Code task" }),
  ).toBeEnabled();
  const stop = page.getByRole("button", { name: "Stop", exact: true });
  await expect(stop).toBeVisible();
  await expect(stop).toBeDisabled();
  expect(await commandPayloads(page, "code_turn_interrupt")).toEqual([]);

  await emitCodeEvent(
    page,
    codeEvent(1, "turn/started", {
      turnId: CREATED_TURN_ID,
      payload: {
        turn: { id: CREATED_TURN_ID, status: "inProgress" },
      },
    }),
  );
  await expect(stop).toBeEnabled();
  await stop.click();

  await waitForCommand(page, "code_turn_interrupt");
  expect(await commandPayload(page, "code_turn_interrupt")).toEqual({
    input: {
      scope: SCOPE,
      threadId: THREAD_ID,
      turnId: CREATED_TURN_ID,
    },
  });
  await expect(stop).toBeHidden();
  await expect(
    page.getByRole("textbox", { name: "Steer active Code task" }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("textbox", { name: "Message Code task" }),
  ).toBeEnabled();
});

test("restores and resumes a managed-worktree task after app re-entry", async ({
  page,
}) => {
  await installMockBridge(page, { schoolxCodeWorkspace: true });
  await openBoundThread(page);
  await createManagedTask(page);

  await page.reload({ waitUntil: "domcontentloaded" });
  await page.waitForFunction(
    () => Array.isArray(window.__BUZZ_E2E_COMMANDS__),
    undefined,
    { timeout: 15_000 },
  );
  await expect(page).toHaveURL(new RegExp(`threadId=${CREATED_THREAD_ID}`));
  await expect(codeRuntimeReadyLabel(page)).toBeVisible({
    timeout: 10_000,
  });
  await waitForCommand(page, "code_threads_list");
  expect(await commandPayload(page, "code_threads_list")).toEqual({
    input: { scope: SCOPE },
  });
  await waitForCommandCount(page, "code_thread_resume", 1);
  expect(await commandPayload(page, "code_thread_resume")).toEqual({
    input: {
      scope: SCOPE,
      threadId: CREATED_THREAD_ID,
      model: null,
    },
  });

  const restoredThread = page.getByTestId(`code-thread-${CREATED_THREAD_ID}`);
  await expect(restoredThread).toBeVisible();
  await expect(restoredThread).toHaveAttribute("aria-current", "page");
  await expect(
    page.getByRole("heading", { name: "Managed worktree task", exact: true }),
  ).toBeVisible();
  const timeline = page.getByTestId("code-timeline");
  await expect(timeline).toContainText("Start with a request");
  await expect(timeline).not.toContainText(
    "Historical Code response from fixture.",
  );
  await expect(
    page.getByRole("textbox", { name: "Message Code task" }),
  ).toBeEnabled();
});

test("refreshes only exact selected-thread Changes across event and generation identities", async ({
  page,
}) => {
  await page.clock.setFixedTime(new Date("2026-08-14T00:00:00Z"));
  const nextGenerationChange = codeEvent(6, "turn/diff/updated", {
    payload: { diff: "same sequence in the next generation" },
    runtimeGeneration: 8,
  });
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeEventBacklogs: [
      eventBacklog([]),
      eventBacklog([nextGenerationChange], false, 8),
    ],
  });
  await openBoundThread(page);

  const changesInspector = page.getByTestId("code-changes-inspector");
  const expectedChangesPayload = {
    input: { scope: SCOPE, threadId: THREAD_ID },
  };
  const expectChangesCalls = async (count: number) => {
    await waitForCommandCount(page, "code_thread_changes", count);
    await waitForTwoAnimationFrames(page);
    expect(await commandPayloads(page, "code_thread_changes")).toEqual(
      Array.from({ length: count }, () => expectedChangesPayload),
    );
  };
  await expect(changesInspector).toBeVisible();
  await expectChangesCalls(1);

  await emitCodeEvent(
    page,
    codeEvent(1, "turn/diff/updated", {
      payload: { diff: "other thread update" },
      threadId: "thread-e2e-other",
      turnId: "turn-e2e-other",
    }),
  );
  await waitForTwoAnimationFrames(page);
  expect(await commandPayloads(page, "code_thread_changes")).toEqual([
    expectedChangesPayload,
  ]);

  await emitCodeEvent(
    page,
    codeEvent(2, "warning", {
      payload: { message: "Selected-thread non-change marker." },
    }),
  );
  await expect(page.getByTestId("code-timeline")).toContainText(
    "Selected-thread non-change marker.",
  );
  await waitForTwoAnimationFrames(page);
  expect(await commandPayloads(page, "code_thread_changes")).toEqual([
    expectedChangesPayload,
  ]);

  await emitCodeEvent(
    page,
    codeEvent(3, "turn/diff/updated", {
      payload: { diff: "selected thread update" },
    }),
  );
  await expect(page.getByTestId("code-timeline")).toContainText(
    "Codex reported a file update.",
  );
  await expectChangesCalls(2);

  await emitCodeEvent(
    page,
    codeEvent(4, "item/fileChange/patchUpdated", {
      itemId: "item-live-file-change",
      payload: {
        changes: [
          {
            path: "desktop/src/features/code/ui/CodeWorkspaceScreen.tsx",
            kind: { type: "update" },
          },
        ],
      },
    }),
  );
  await expectChangesCalls(3);

  await emitCodeEvent(
    page,
    codeEvent(5, "turn/completed", {
      payload: {
        turn: { id: TURN_ID, status: "completed", items: [], error: null },
      },
    }),
  );
  await expectChangesCalls(4);

  await page.getByRole("button", { name: "Hide Changes inspector" }).click();
  await expect(changesInspector).toBeHidden();
  await emitCodeEvent(
    page,
    codeEvent(6, "item/fileChange/patchUpdated", {
      itemId: "item-closed-file-change",
      payload: {
        changes: [
          {
            path: "desktop/src/features/code/ui/CodeChangesPanel.tsx",
            kind: { type: "update" },
          },
        ],
      },
    }),
  );
  await waitForTwoAnimationFrames(page);
  expect(await commandPayloads(page, "code_thread_changes")).toHaveLength(4);

  await page.getByRole("button", { name: "Show Changes inspector" }).click();
  await expect(changesInspector).toBeVisible();
  await expectChangesCalls(5);

  await page.evaluate(() => {
    const crashRuntime = window.__BUZZ_E2E_CRASH_CODE_RUNTIME__;
    if (!crashRuntime)
      throw new Error("Code runtime crash seam is unavailable");
    crashRuntime("Fixture app-server exited before a generation rollover.");
  });
  await page
    .getByRole("button", { name: "Refresh Codex runtime status" })
    .click();
  await expect(
    page.getByText("Runtime unavailable", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(codeRuntimeReadyLabel(page)).toBeVisible();
  await waitForCommandCount(page, "code_runtime_events", 2);
  expect(await commandPayload(page, "code_runtime_events")).toEqual({
    scope: SCOPE,
    runtimeGeneration: 8,
    afterSequence: 0,
  });
  await expectChangesCalls(6);

  await emitCodeEvent(
    page,
    codeEvent(7, "item/fileChange/patchUpdated", {
      itemId: "item-next-generation-file-change",
      payload: { changes: [] },
      runtimeGeneration: 8,
    }),
  );
  await expectChangesCalls(7);

  await emitCodeEvent(
    page,
    codeEvent(8, "item/fileChange/patchUpdated", {
      itemId: "item-stale-file-change",
      payload: { changes: [] },
      runtimeGeneration: 7,
    }),
  );
  await waitForTwoAnimationFrames(page);
  expect(await commandPayloads(page, "code_thread_changes")).toHaveLength(7);
});

test("projects normalized plan, command, file, approval, and completed-turn events", async ({
  page,
}) => {
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeEventBacklogs: [eventBacklog(NORMALIZED_TIMELINE_EVENTS)],
  });
  await openBoundThread(page);

  const timeline = page.getByTestId("code-timeline");
  await expect(timeline).toContainText(
    "Verify the normalized Code event pipeline.",
  );
  await expect(timeline).toContainText("Inspect the focused adapter");
  await expect(timeline).toContainText("pnpm test:e2e schoolx-code");
  await expect(timeline).toContainText("3 focused tests passed");
  await expect(timeline).toContainText(
    "desktop/src/features/code/ui/CodeTimeline.tsx",
  );

  const commandApproval = page.getByTestId("code-approval-approval-command");
  const fileApproval = page.getByTestId("code-approval-approval-file");
  const permissionDeclineApproval = page.getByTestId(
    "code-approval-approval-permission-decline",
  );
  const permissionTurnApproval = page.getByTestId(
    "code-approval-approval-permission-turn",
  );
  const permissionSessionApproval = page.getByTestId(
    "code-approval-approval-permission-session",
  );
  await expect(commandApproval).toContainText("git status --short");
  await expect(fileApproval).toContainText("Apply the normalized file update.");
  await expect(permissionDeclineApproval).toContainText(
    "Decline the standalone network permission request.",
  );
  await expect(permissionTurnApproval).toContainText(
    "Allow this read permission for the current turn.",
  );
  await expect(permissionSessionApproval).toContainText(
    "Allow this write permission for the session.",
  );

  await commandApproval.getByRole("button", { name: "Allow once" }).click();
  await fileApproval.getByRole("button", { name: "Decline" }).click();
  await permissionDeclineApproval
    .getByRole("button", { name: "Decline" })
    .click();
  await permissionTurnApproval
    .getByRole("button", { name: "Allow once" })
    .click();
  await permissionSessionApproval
    .getByRole("button", { name: "Allow for session" })
    .click();
  await waitForCommandCount(page, "code_approval_respond", 5);
  expect(await commandPayloads(page, "code_approval_respond")).toEqual([
    {
      input: {
        runtimeGeneration: 7,
        requestId: 41,
        scope: SCOPE,
        threadId: THREAD_ID,
        turnId: TURN_ID,
        response: { type: "decision", decision: "accept" },
      },
    },
    {
      input: {
        runtimeGeneration: 7,
        requestId: "41",
        scope: SCOPE,
        threadId: THREAD_ID,
        turnId: TURN_ID,
        response: { type: "decision", decision: "decline" },
      },
    },
    {
      input: {
        runtimeGeneration: 7,
        requestId: "permission-decline",
        scope: SCOPE,
        threadId: THREAD_ID,
        turnId: TURN_ID,
        response: {
          type: "permissions",
          intent: "decline",
          scope: "turn",
        },
      },
    },
    {
      input: {
        runtimeGeneration: 7,
        requestId: 42,
        scope: SCOPE,
        threadId: THREAD_ID,
        turnId: TURN_ID,
        response: {
          type: "permissions",
          intent: "grant",
          scope: "turn",
        },
      },
    },
    {
      input: {
        runtimeGeneration: 7,
        requestId: "permission-session",
        scope: SCOPE,
        threadId: THREAD_ID,
        turnId: TURN_ID,
        response: {
          type: "permissions",
          intent: "grant",
          scope: "session",
        },
      },
    },
  ]);

  const completedLabelsBefore = await timeline
    .getByText("completed", { exact: true })
    .count();
  await emitCodeEvent(
    page,
    codeEvent(13, "turn/completed", {
      payload: {
        turn: { id: TURN_ID, status: "completed", items: [], error: null },
      },
    }),
  );
  await expect
    .poll(() => timeline.getByText("completed", { exact: true }).count())
    .toBe(completedLabelsBefore + 1);
});

test("restores an evicted approval from the authoritative runtime checkpoint", async ({
  page,
}) => {
  const replayApproval = approvalEvent({
    approvalKind: "commandExecution",
    itemId: "approval-replay",
    kind: "item/commandExecution/requestApproval",
    request: {
      command: "cargo test -p buzz-core",
      reason: "Replay this exact pending request.",
    },
    requestId: "replay-request-1",
    sequence: 2,
  });
  const replayEvents = [
    codeEvent(1, "turn/started", {
      payload: { turn: { id: TURN_ID, status: "inProgress" } },
    }),
    replayApproval,
  ];
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeEventBacklogs: [
      eventBacklog(replayEvents, true, 7, {
        runtimeGeneration: 7,
        sequenceWatermark: 2,
        activeTurns: [
          {
            threadId: THREAD_ID,
            turnId: TURN_ID,
            status: "inProgress",
            startedSequence: 1,
          },
        ],
        pendingApprovals: [{ event: replayApproval, respondable: true }],
      }),
    ],
  });
  await openBoundThread(page);

  const approval = page.getByTestId("code-approval-approval-replay");
  await expect(
    approval.getByRole("button", { name: "Allow once" }),
  ).toBeEnabled();
  await waitForCommandCount(page, "code_runtime_events", 1);
  expect(await commandPayload(page, "code_runtime_events")).toEqual({
    scope: SCOPE,
    runtimeGeneration: 7,
    afterSequence: 0,
  });
  await expect(page.getByRole("button", { name: "Sync activity" })).toHaveCount(
    0,
  );

  await approval.getByRole("button", { name: "Allow once" }).click();
  await waitForCommand(page, "code_approval_respond");
  expect(await commandPayload(page, "code_approval_respond")).toEqual({
    input: {
      runtimeGeneration: 7,
      requestId: "replay-request-1",
      scope: SCOPE,
      threadId: THREAD_ID,
      turnId: TURN_ID,
      response: { type: "decision", decision: "accept" },
    },
  });
});

test("recovers a crashed runtime with a new generation and full replay", async ({
  page,
}) => {
  const oldGenerationApproval = approvalEvent({
    approvalKind: "commandExecution",
    itemId: "approval-before-crash",
    kind: "item/commandExecution/requestApproval",
    request: { command: "cargo test -p buzz-core" },
    requestId: "before-crash",
    sequence: 1,
  });
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeEventBacklogs: [
      eventBacklog([oldGenerationApproval]),
      eventBacklog([], false, 8),
    ],
  });
  await openBoundThread(page);

  const staleApproval = page.getByTestId("code-approval-approval-before-crash");
  await expect(staleApproval).toBeVisible();
  await page.evaluate(() => {
    const crashRuntime = window.__BUZZ_E2E_CRASH_CODE_RUNTIME__;
    if (!crashRuntime)
      throw new Error("Code runtime crash seam is unavailable");
    crashRuntime("Fixture app-server exited with status 70.");
  });
  await page
    .getByRole("button", { name: "Refresh Codex runtime status" })
    .click();

  await expect(
    page.getByText("Runtime unavailable", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Fixture app-server exited with status 70."),
  ).toBeVisible();
  await expect(staleApproval).toBeHidden();
  expect(await commandPayloads(page, "code_approval_respond")).toEqual([]);

  await page.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(codeRuntimeReadyLabel(page)).toBeVisible();
  await waitForCommandCount(page, "code_runtime_start", 2);
  await waitForCommandCount(page, "code_runtime_events", 2);
  expect(await commandPayload(page, "code_runtime_events")).toEqual({
    scope: SCOPE,
    runtimeGeneration: 8,
    afterSequence: 0,
  });
  await waitForCommandCount(page, "code_thread_resume", 2);
  await expect(staleApproval).toBeHidden();
  expect(await commandPayloads(page, "code_approval_respond")).toEqual([]);
});

test("keeps the 800x500 dark workspace keyboard-operable with reduced motion", async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 500 });
  await page.emulateMedia({ colorScheme: "dark", reducedMotion: "reduce" });
  const narrowEvents = [
    codeEvent(1, "item/started", {
      itemId: "item-narrow-plan",
      payload: {
        item: {
          id: "item-narrow-plan",
          type: "plan",
          text: "Keep the narrow Code workspace keyboard accessible.",
          steps: [{ step: "Verify reduced motion", status: "inProgress" }],
        },
      },
    }),
    approvalEvent({
      approvalKind: "commandExecution",
      itemId: "approval-keyboard",
      kind: "item/commandExecution/requestApproval",
      request: { command: "pnpm check:px-text" },
      requestId: "keyboard-request",
      sequence: 2,
    }),
  ];
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeEventBacklogs: [eventBacklog(narrowEvents)],
  });
  await openBoundThread(page, true);

  await expect
    .poll(() =>
      page.evaluate(() => document.documentElement.classList.contains("dark")),
    )
    .toBe(true);
  const timeline = page.getByTestId("code-timeline");
  const initialTimelineBox = await timeline.boundingBox();
  expect(initialTimelineBox).not.toBeNull();
  expect(
    (initialTimelineBox?.x ?? 0) + (initialTimelineBox?.width ?? 0),
  ).toBeLessThanOrEqual(800);

  const streamingIndicator = timeline.locator("svg.animate-spin").first();
  await expect(streamingIndicator).toBeVisible();
  expect(
    await streamingIndicator.evaluate(
      (element) => getComputedStyle(element).animationName,
    ),
  ).toBe("none");

  const showChanges = page.getByRole("button", {
    name: "Show Changes inspector",
  });
  await activate(page, showChanges, true);
  const changesInspector = page.getByTestId("code-changes-inspector");
  await expect(changesInspector).toBeVisible();
  const inspectorBox = await changesInspector.boundingBox();
  expect(inspectorBox).not.toBeNull();
  expect(inspectorBox?.x ?? -1).toBeGreaterThanOrEqual(0);
  expect(
    (inspectorBox?.x ?? 0) + (inspectorBox?.width ?? 0),
  ).toBeLessThanOrEqual(800);
  await changesInspector
    .getByRole("button", { name: "Close Changes inspector" })
    .focus();
  await page.keyboard.press("Escape");
  await expect(changesInspector).toBeHidden();

  const hideSidebar = page.getByRole("button", { name: "Hide task sidebar" });
  await activate(page, hideSidebar, true);
  const showSidebar = page.getByRole("button", { name: "Show task sidebar" });
  await expect(showSidebar).toBeVisible();
  const expandedTimelineBox = await timeline.boundingBox();
  expect(expandedTimelineBox).not.toBeNull();
  expect(expandedTimelineBox?.width ?? 0).toBeGreaterThan(
    initialTimelineBox?.width ?? 0,
  );

  const allowOnce = page
    .getByTestId("code-approval-approval-keyboard")
    .getByRole("button", { name: "Allow once" });
  await activate(page, allowOnce, true);
  await waitForCommand(page, "code_approval_respond");
  expect(await commandPayload(page, "code_approval_respond")).toEqual({
    input: {
      runtimeGeneration: 7,
      requestId: "keyboard-request",
      scope: SCOPE,
      threadId: THREAD_ID,
      turnId: TURN_ID,
      response: { type: "decision", decision: "accept" },
    },
  });
});

test("searches exact task metadata and renames the selected bound thread", async ({
  page,
}) => {
  await installMockBridge(page, { schoolxCodeWorkspace: true });
  await openCodeWorkspace(page);
  await createManagedTask(page);

  const existingThread = page.getByTestId(`code-thread-${THREAD_ID}`);
  const createdThread = page.getByTestId(`code-thread-${CREATED_THREAD_ID}`);
  const search = page.getByTestId("code-thread-search");
  await expect(search).toHaveAttribute("aria-label", "Search Code tasks");

  await search.fill("Code shell task");
  await expect(existingThread).toBeVisible();
  await expect(createdThread).toBeHidden();

  await search.fill("Continue the managed-worktree task");
  await expect(createdThread).toBeVisible();
  await expect(existingThread).toBeHidden();

  await search.fill(CREATED_THREAD_ID);
  await expect(createdThread).toBeVisible();
  await expect(existingThread).toBeHidden();

  const selectedUrl = page.url();
  await search.fill("no task matches this query");
  await expect(existingThread).toBeHidden();
  await expect(createdThread).toBeHidden();
  await expect(
    page.getByText("No matching tasks", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Managed worktree task", exact: true }),
  ).toBeVisible();
  expect(page.url()).toBe(selectedUrl);

  await page.getByRole("button", { name: "Clear search" }).click();
  await expect(search).toHaveValue("");
  await expect(existingThread).toBeVisible();
  await expect(createdThread).toBeVisible();
  await expect(createdThread).toHaveAttribute("aria-current", "page");
  expect(page.url()).toBe(selectedUrl);

  await page.getByTestId(`code-thread-actions-${CREATED_THREAD_ID}`).click();
  await page.getByRole("menuitem", { name: "Rename task" }).click();
  const renameDialog = page.getByTestId("code-thread-rename-dialog");
  await expect(renameDialog).toBeVisible();
  const nameInput = renameDialog.getByRole("textbox", { name: "Task name" });
  await expect(nameInput).toHaveValue("Managed worktree task");
  await nameInput.fill("Renamed exact-bound task");
  await renameDialog.getByRole("button", { name: "Save" }).click();

  await waitForCommandCount(page, "code_thread_rename", 1);
  expect(await commandPayload(page, "code_thread_rename")).toEqual({
    input: {
      scope: SCOPE,
      threadId: CREATED_THREAD_ID,
      name: "Renamed exact-bound task",
    },
  });
  await expect(renameDialog).toBeHidden();
  await expect(createdThread).toContainText("Renamed exact-bound task");
  await expect(
    page.getByRole("heading", {
      name: "Renamed exact-bound task",
      exact: true,
    }),
  ).toBeVisible();
  expect(page.url()).toBe(selectedUrl);

  await page.evaluate(() => {
    const mock = window.__BUZZ_E2E__?.mock;
    if (!mock) throw new Error("Mock bridge configuration is unavailable");
    mock.schoolxCodeRenameErrors = [
      null,
      "Fixture rejected the bound-thread rename.",
    ];
  });
  await page.getByTestId(`code-thread-actions-${CREATED_THREAD_ID}`).click();
  await page.getByRole("menuitem", { name: "Rename task" }).click();
  await expect(nameInput).toHaveValue("Renamed exact-bound task");
  await nameInput.fill("Rejected task name");
  await renameDialog.getByRole("button", { name: "Save" }).click();

  await waitForCommandCount(page, "code_thread_rename", 2);
  expect(await commandPayload(page, "code_thread_rename")).toEqual({
    input: {
      scope: SCOPE,
      threadId: CREATED_THREAD_ID,
      name: "Rejected task name",
    },
  });
  await expect(renameDialog).toBeVisible();
  await expect(renameDialog.getByRole("alert")).toHaveText(
    "Fixture rejected the bound-thread rename.",
  );
  await expect(createdThread).toContainText("Renamed exact-bound task");
  expect(page.url()).toBe(selectedUrl);
  await renameDialog.getByRole("button", { name: "Cancel" }).click();
  await expect(renameDialog).toBeHidden();
  await expect(
    page.getByRole("heading", {
      name: "Renamed exact-bound task",
      exact: true,
    }),
  ).toBeVisible();

  expect(
    await invokeMockCommandError(page, "code_thread_rename", {
      input: {
        scope: SCOPE,
        threadId: CREATED_THREAD_ID,
        name: "Must not cross the trust boundary",
        cwd: "/tmp/untrusted",
      },
    }),
  ).toEqual({
    message: "Thread rename input crossed its native trust boundary",
    payload: null,
  });
  expect(
    await invokeMockCommandError(page, "code_thread_rename", {
      input: {
        scope: SCOPE,
        threadId: "thread-outside-this-binding",
        name: "Must remain bound",
      },
    }),
  ).toEqual(THREAD_SCOPE_FAILURE);
});

test("forks an active managed task into a fresh selected destination", async ({
  page,
}) => {
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeForkDelayMs: 250,
  });
  await openCodeWorkspace(page);

  await page.getByTestId(`code-thread-actions-${THREAD_ID}`).click();
  await expect(
    page.getByRole("menuitem", { name: "Fork task" }),
  ).toBeDisabled();
  await page.keyboard.press("Escape");

  await createManagedTask(page);
  const source = page.getByTestId(`code-thread-${CREATED_THREAD_ID}`);
  const sourceActions = page.getByTestId(
    `code-thread-actions-${CREATED_THREAD_ID}`,
  );
  const listCountBefore = (await commandPayloads(page, "code_threads_list"))
    .length;
  const preparationCountBefore = (
    await commandPayloads(page, "code_thread_preparations_list")
  ).length;
  await sourceActions.click();
  await page.getByRole("menuitem", { name: "Fork task" }).click();

  await waitForCommandCount(page, "code_thread_fork", 1);
  expect(await commandPayload(page, "code_thread_fork")).toEqual({
    input: { scope: SCOPE, threadId: CREATED_THREAD_ID },
  });
  await expect(sourceActions).toHaveAttribute("aria-busy", "true");
  await expect(source).toHaveAttribute("aria-current", "page");
  await expect(page).toHaveURL(new RegExp(`threadId=${CREATED_THREAD_ID}`));

  const destination = page.getByTestId(`code-thread-${FORKED_THREAD_ID}`);
  await expect(destination).toBeVisible();
  await expect(destination).toHaveAttribute("aria-current", "page");
  await expect(source).not.toHaveAttribute("aria-current", "page");
  await expect(page).toHaveURL(new RegExp(`threadId=${FORKED_THREAD_ID}`));
  await expect(destination).toContainText("Forked managed worktree task");
  await expect(destination).toContainText("Managed worktree");
  await expect(
    page.getByRole("heading", {
      name: "Forked managed worktree task",
      exact: true,
    }),
  ).toBeVisible();
  expect((await commandPayloads(page, "code_threads_list")).length).toBe(
    listCountBefore,
  );
  expect(
    (await commandPayloads(page, "code_thread_preparations_list")).length,
  ).toBeGreaterThan(preparationCountBefore);
  expect(
    (await commandPayloads(page, "code_thread_resume")).filter(
      ({ input }) =>
        (input as { threadId?: string } | undefined)?.threadId ===
        FORKED_THREAD_ID,
    ),
  ).toEqual([]);
  expect(await commandPayloads(page, "code_terminal_open")).toEqual([]);

  expect(
    await invokeMockCommandError(page, "code_thread_fork", {
      input: {
        scope: SCOPE,
        threadId: CREATED_THREAD_ID,
        cwd: "/tmp/untrusted",
      },
    }),
  ).toEqual({
    message: "Thread fork crossed its native trust boundary",
    payload: null,
  });
});

test("keeps a failed fork on its source and continues its exact preparation", async ({
  page,
}) => {
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeForkErrors: ["Fixture left the fork definitely not sent."],
  });
  await openCodeWorkspace(page);
  await createManagedTask(page);

  const source = page.getByTestId(`code-thread-${CREATED_THREAD_ID}`);
  await page.getByTestId(`code-thread-actions-${CREATED_THREAD_ID}`).click();
  await page.getByRole("menuitem", { name: "Fork task" }).click();
  await waitForCommandCount(page, "code_thread_fork", 1);

  await expect(source).toHaveAttribute("aria-current", "page");
  await expect(page).toHaveURL(new RegExp(`threadId=${CREATED_THREAD_ID}`));
  await expect(page.getByRole("alert")).toContainText(
    "Fixture left the fork definitely not sent.",
  );
  await expect(page.getByTestId(`code-thread-${FORKED_THREAD_ID}`)).toHaveCount(
    0,
  );

  const preparation = page.getByTestId(
    `code-preparation-${FORK_PREPARATION_ID}`,
  );
  await expect(preparation).toContainText("Prepared fork");
  const continueFork = preparation.getByRole("button", {
    name: "Continue fork",
  });
  await expect(continueFork).toBeEnabled();

  await page.getByTestId(`code-thread-actions-${CREATED_THREAD_ID}`).click();
  await expect(
    page.getByRole("menuitem", { name: "Fork task" }),
  ).toBeDisabled();
  await page.keyboard.press("Escape");
  await continueFork.click();

  await waitForCommandCount(page, "code_thread_binding_recover", 1);
  expect(await commandPayload(page, "code_thread_binding_recover")).toEqual({
    input: {
      scope: SCOPE,
      preparationId: FORK_PREPARATION_ID,
      model: null,
    },
  });
  const destination = page.getByTestId(`code-thread-${FORKED_THREAD_ID}`);
  await expect(destination).toHaveAttribute("aria-current", "page");
  await expect(source).not.toHaveAttribute("aria-current", "page");
  await expect(preparation).toHaveCount(0);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(page).toHaveURL(new RegExp(`threadId=${FORKED_THREAD_ID}`));
  expect(await commandPayloads(page, "code_thread_fork")).toHaveLength(1);
});

test("archives the selected task in place and keeps archived work read-only", async ({
  page,
}) => {
  await installMockBridge(page, { schoolxCodeWorkspace: true });
  await openBoundThread(page);
  await createManagedTask(page);

  const existingThread = page.getByTestId(`code-thread-${THREAD_ID}`);
  const createdThread = page.getByTestId(`code-thread-${CREATED_THREAD_ID}`);
  await existingThread.click();
  await expect(page).toHaveURL(new RegExp(`threadId=${THREAD_ID}`));
  await expect(existingThread).toHaveAttribute("aria-current", "page");
  const selectedUrl = page.url();
  const resumeCount = (await commandPayloads(page, "code_thread_resume"))
    .length;

  await page.getByTestId(`code-thread-actions-${THREAD_ID}`).click();
  await page.getByRole("menuitem", { name: "Archive task" }).click();
  const archiveDialog = page.getByTestId("code-thread-archive-dialog");
  await expect(archiveDialog).toBeVisible();
  await expect(archiveDialog).toContainText(
    "The binding, worktree, and local changes are preserved",
  );
  await archiveDialog.getByRole("button", { name: "Archive task" }).click();

  await waitForCommandCount(page, "code_thread_archive", 1);
  expect(await commandPayload(page, "code_thread_archive")).toEqual({
    input: { scope: SCOPE, threadId: THREAD_ID },
  });
  await expect(
    page.getByTestId(`code-thread-lifecycle-${THREAD_ID}`),
  ).toHaveText("Archived");
  await expect(page.getByTestId("code-thread-lifecycle-notice")).toContainText(
    "This task is archived and read-only",
  );
  expect(page.url()).toBe(selectedUrl);
  await expect(existingThread).toHaveAttribute("aria-current", "page");
  await expect(createdThread).not.toHaveAttribute("aria-current", "page");
  await expect(page.getByTestId("code-timeline")).toContainText(
    "Historical Code response from fixture.",
  );
  await expect(page.getByTestId("code-changes-inspector")).toBeVisible();

  await expect(
    page.getByRole("textbox", { name: "Message Code task" }),
  ).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Retry task" })).toHaveCount(0);
  await expect(page.getByTestId("code-terminal-toggle")).toHaveCount(0);
  await page.keyboard.press("ControlOrMeta+j");
  await waitForTwoAnimationFrames(page);
  await waitForCommandCount(page, "code_thread_resume", resumeCount);
  expect(await commandPayloads(page, "code_turn_start")).toEqual([]);
  expect(await commandPayloads(page, "code_turn_steer")).toEqual([]);
  expect(await commandPayloads(page, "code_terminal_open")).toEqual([]);

  await page.getByTestId(`code-thread-actions-${THREAD_ID}`).click();
  await page.getByRole("menuitem", { name: "Rename task" }).click();
  const renameDialog = page.getByTestId("code-thread-rename-dialog");
  await expect(renameDialog).toBeVisible();
  await renameDialog.getByRole("button", { name: "Cancel" }).click();
  await page.getByTestId(`code-thread-actions-${THREAD_ID}`).click();
  await expect(
    page.getByRole("menuitem", { name: "Unarchive task" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
});

test("unarchives the same route, resumes automatically, and leaves its PTY closed", async ({
  page,
}) => {
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeThreadLifecycles: { [THREAD_ID]: "archived" },
  });
  await openCodeWorkspace(page);

  const thread = page.getByTestId(`code-thread-${THREAD_ID}`);
  await thread.click();
  await expect(page).toHaveURL(new RegExp(`threadId=${THREAD_ID}`));
  const selectedUrl = page.url();
  await expect(page.getByTestId("code-timeline")).toContainText(
    "Historical Code response from fixture.",
  );
  expect(await commandPayloads(page, "code_thread_resume")).toEqual([]);
  expect(await commandPayloads(page, "code_terminal_open")).toEqual([]);

  const notice = page.getByTestId("code-thread-lifecycle-notice");
  await notice.getByRole("button", { name: "Unarchive task" }).click();
  await waitForCommandCount(page, "code_thread_unarchive", 1);
  expect(await commandPayload(page, "code_thread_unarchive")).toEqual({
    input: { scope: SCOPE, threadId: THREAD_ID },
  });
  await waitForCommandCount(page, "code_thread_resume", 1);
  expect(await commandPayload(page, "code_thread_resume")).toEqual({
    input: { scope: SCOPE, threadId: THREAD_ID, model: null },
  });
  await expect(
    page.getByRole("textbox", { name: "Message Code task" }),
  ).toBeEnabled();
  await expect(
    page.getByTestId(`code-thread-lifecycle-${THREAD_ID}`),
  ).toHaveText("Active");
  await expect(page.getByTestId("code-terminal-toggle")).toBeVisible();
  await expect(page.getByTestId("code-terminal-drawer")).toBeHidden();
  expect(await commandPayloads(page, "code_terminal_open")).toEqual([]);
  expect(page.url()).toBe(selectedUrl);
});

test("keeps an unknown lifecycle read-only and exposes refresh only", async ({
  page,
}) => {
  await installMockBridge(page, {
    schoolxCodeWorkspace: true,
    schoolxCodeThreadLifecycles: { [THREAD_ID]: "unknown" },
  });
  await openCodeWorkspace(page);

  const thread = page.getByTestId(`code-thread-${THREAD_ID}`);
  await thread.click();
  await expect(page).toHaveURL(new RegExp(`threadId=${THREAD_ID}`));
  await expect(
    page.getByTestId(`code-thread-lifecycle-${THREAD_ID}`),
  ).toHaveText("Status unknown");
  await expect(page.getByTestId("code-timeline")).toContainText(
    "Historical Code response from fixture.",
  );
  await expect(
    page.getByTestId(`code-thread-actions-${THREAD_ID}`),
  ).toHaveCount(0);
  await expect(
    page.getByRole("textbox", { name: "Message Code task" }),
  ).toHaveCount(0);
  await expect(page.getByTestId("code-terminal-toggle")).toHaveCount(0);
  await expect(page.getByTestId("code-changes-inspector")).toHaveCount(0);

  const listCount = (await commandPayloads(page, "code_threads_list")).length;
  const notice = page.getByTestId("code-thread-lifecycle-notice");
  const refresh = notice.getByRole("button", { name: "Refresh status" });
  await expect(refresh).toBeEnabled();
  await refresh.click();
  await expect
    .poll(async () => (await commandPayloads(page, "code_threads_list")).length)
    .toBeGreaterThan(listCount);
  await page.keyboard.press("ControlOrMeta+j");
  await waitForTwoAnimationFrames(page);

  expect(await commandPayloads(page, "code_thread_resume")).toEqual([]);
  expect(await commandPayloads(page, "code_turn_start")).toEqual([]);
  expect(await commandPayloads(page, "code_turn_steer")).toEqual([]);
  expect(await commandPayloads(page, "code_approval_respond")).toEqual([]);
  expect(await commandPayloads(page, "code_terminal_open")).toEqual([]);
  expect(await commandPayloads(page, "code_thread_rename")).toEqual([]);
  expect(await commandPayloads(page, "code_thread_archive")).toEqual([]);
  expect(await commandPayloads(page, "code_thread_unarchive")).toEqual([]);
});

test("owns an exact bound-thread PTY across Cmd-J hide, resize, input, and terminate", async ({
  page,
}) => {
  await installMockBridge(page, { schoolxCodeWorkspace: true });
  await openBoundThread(page);

  const composer = page.getByRole("textbox", { name: "Message Code task" });
  await composer.focus();
  await page.keyboard.press("ControlOrMeta+j");

  const drawer = page.getByTestId("code-terminal-drawer");
  await expect(drawer).toBeVisible();
  await expect(drawer.getByRole("heading", { name: "Terminal" })).toBeVisible();
  await waitForCommandCount(page, "code_terminal_open", 1);

  const openPayload = (await commandPayload(page, "code_terminal_open")) as {
    input: {
      scope: typeof SCOPE;
      threadId: string;
      cols: number;
      rows: number;
    };
    onEvent: string;
  };
  expect(openPayload.input.scope).toEqual(SCOPE);
  expect(openPayload.input.threadId).toBe(THREAD_ID);
  expect(openPayload.input.cols).toBeGreaterThan(0);
  expect(openPayload.input.rows).toBeGreaterThan(0);
  expect(Object.keys(openPayload.input).sort()).toEqual(
    ["cols", "rows", "scope", "threadId"].sort(),
  );
  expect(openPayload.onEvent).toMatch(/^__CHANNEL__:\d+$/);

  const terminal = drawer.getByTestId("code-terminal");
  await expect(terminal.locator(".xterm")).toBeVisible();
  await terminal.click();
  await page.keyboard.type("pwd");
  await page.keyboard.press("Enter");
  await page.keyboard.press("Escape");
  await waitForCommand(page, "code_terminal_stdin");
  await expect(drawer).toBeVisible();

  const stdinPayloads = (await commandPayloads(
    page,
    "code_terminal_stdin",
  )) as Array<{
    input: {
      scope: typeof SCOPE;
      threadId: string;
      sessionId: string;
      data: number[];
    };
  }>;
  expect(stdinPayloads.length).toBeGreaterThanOrEqual(3);
  const sessionId = stdinPayloads[0]?.input.sessionId;
  expect(sessionId).toMatch(
    /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
  );
  for (const { input } of stdinPayloads) {
    expect(input.scope).toEqual(SCOPE);
    expect(input.threadId).toBe(THREAD_ID);
    expect(input.sessionId).toBe(sessionId);
    expect(Object.keys(input).sort()).toEqual(
      ["data", "scope", "sessionId", "threadId"].sort(),
    );
  }
  const stdinBytes = stdinPayloads.flatMap(({ input }) => input.data);
  expect(stdinBytes).toEqual(expect.arrayContaining([13, 27, 100, 112, 119]));

  await page.setViewportSize({ width: 1000, height: 650 });
  await waitForCommand(page, "code_terminal_resize");
  const resizePayload = (await commandPayload(
    page,
    "code_terminal_resize",
  )) as {
    input: {
      scope: typeof SCOPE;
      threadId: string;
      sessionId: string;
      cols: number;
      rows: number;
    };
  };
  expect(resizePayload.input.scope).toEqual(SCOPE);
  expect(resizePayload.input.threadId).toBe(THREAD_ID);
  expect(resizePayload.input.sessionId).toBe(sessionId);
  expect(resizePayload.input.cols).toBeGreaterThan(0);
  expect(resizePayload.input.rows).toBeGreaterThan(0);

  await page.keyboard.press("ControlOrMeta+j");
  await expect(drawer).toBeHidden();
  await expect(composer).toBeFocused();
  await waitForCommandCount(page, "code_terminal_open", 1);
  expect(await commandPayloads(page, "code_terminal_terminate")).toEqual([]);

  await page.keyboard.press("ControlOrMeta+j");
  await expect(drawer).toBeVisible();
  await waitForCommandCount(page, "code_terminal_open", 1);

  await page.evaluate(() => {
    const crashRuntime = window.__BUZZ_E2E_CRASH_CODE_RUNTIME__;
    if (!crashRuntime)
      throw new Error("Code runtime crash seam is unavailable");
    crashRuntime("Fixture app-server exited while the user shell stayed open.");
  });
  await page
    .getByRole("button", { name: "Refresh Codex runtime status" })
    .click();
  await expect(
    page.getByText("Runtime unavailable", { exact: true }),
  ).toBeVisible();
  await expect(drawer).toBeVisible();
  expect(await commandPayloads(page, "code_terminal_terminate")).toEqual([]);

  await page.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(codeRuntimeReadyLabel(page)).toBeVisible();
  await expect(drawer).toBeVisible();
  await waitForCommandCount(page, "code_terminal_open", 1);
  expect(await commandPayloads(page, "code_terminal_terminate")).toEqual([]);

  await drawer
    .getByRole("button", { name: "Terminate terminal session" })
    .click();
  await page.getByRole("button", { name: "Terminate session" }).click();
  await waitForCommandCount(page, "code_terminal_terminate", 1);
  expect(await commandPayload(page, "code_terminal_terminate")).toEqual({
    input: { scope: SCOPE, threadId: THREAD_ID, sessionId },
  });
  await expect(drawer).toBeVisible();
  await expect(
    drawer.getByRole("button", { name: "Start terminal" }),
  ).toBeVisible();
});
