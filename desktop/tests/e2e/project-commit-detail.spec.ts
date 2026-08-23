import { expect, test, type Page, type Route } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/project-commit-detail";
const ALIGNMENT_TOLERANCE_PX = 2;
const EMPTY_PROJECT_COORDINATES = [
  `30617:${"deadbeef".repeat(8)}:buzz`,
  `30617:${TEST_IDENTITIES.alice.pubkey}:relay-tools`,
  `30617:${TEST_IDENTITIES.bob.pubkey}:design-system`,
];

// The projects surface is a preview feature — opt in before the app mounts.
// Must run before installMockBridge so React reads the override on mount.
async function enableProjectsFeature(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem(
      "buzz-feature-overrides-v1",
      JSON.stringify({ projects: true }),
    );
  });
}

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

async function openMockApp(page: Page) {
  // Python's preview server can reset Vite's cold parallel ESM fan-out.
  // Serialize built modules while leaving product bootstrap failures visible.
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

test("keeps project loading visible and accessible during delayed relay reads", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page, { projectQueryDelayMs: 500 });

  await openMockApp(page);
  await page.getByTestId("open-projects-view").click();

  await expect(
    page.getByRole("status", { name: "Loading projects", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { level: 1, name: "Projects" }),
  ).toBeVisible();

  await page.getByRole("button", { name: "Repositories", exact: true }).click();
  const projectEntry = page
    .locator(
      '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
    )
    .first();
  await expect(projectEntry).toBeVisible();
  await projectEntry
    .getByRole("button", { name: "View buzz", exact: true })
    .click();

  await expect(
    page.getByRole("status", { name: "Loading project", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("tab", { name: "Overview", exact: true }),
  ).toBeVisible();
});

test("empty projects state exposes project creation", async ({ page }) => {
  await enableProjectsFeature(page);
  await page.addInitScript((coordinates) => {
    window.localStorage.setItem(
      "buzz.projects.hidden-cards.v1",
      JSON.stringify(coordinates),
    );
  }, EMPTY_PROJECT_COORDINATES);
  await installMockBridge(page);

  await openMockApp(page);
  await page.getByTestId("open-projects-view").click();

  await expect(
    page.getByText("No projects yet", { exact: true }),
  ).toBeVisible();
  const createProject = page.getByRole("button", {
    name: "Create project",
    exact: true,
  });
  await expect(createProject).toBeVisible();
  await createProject.click();

  await expect(page.getByTestId("create-project-dialog")).toBeVisible();
  await expect(page.getByTestId("create-project-name")).toBeFocused();
});

test("creates a relay project with a dedicated discussion channel", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await page.addInitScript((coordinates) => {
    window.localStorage.setItem(
      "buzz.projects.hidden-cards.v1",
      JSON.stringify(coordinates),
    );
  }, EMPTY_PROJECT_COORDINATES);
  await installMockBridge(page);

  await openMockApp(page);
  await page.getByTestId("open-projects-view").click();
  await page
    .getByRole("button", { name: "Create project", exact: true })
    .click();

  await page.getByTestId("create-project-name").fill("첫 번째 프로젝트");
  await expect(page.getByTestId("create-project-repository-id")).toHaveValue(
    "",
  );
  await page.getByTestId("create-project-repository-id").fill("first-project");
  await page
    .getByTestId("create-project-description")
    .fill("SchoolX에서 시작한 첫 프로젝트");
  await page.getByTestId("create-project-submit").click();

  await expect(page.getByTestId("create-project-dialog")).toBeHidden();
  await expect(page.getByTestId("project-card-first-project")).toBeVisible();

  const announcement = await page.evaluate(() =>
    window.__BUZZ_E2E_SIGNED_EVENTS__?.find((event) => event.kind === 30617),
  );
  expect(announcement?.content).toBe("SchoolX에서 시작한 첫 프로젝트");
  expect(announcement?.tags).toContainEqual(["d", "first-project"]);
  expect(announcement?.tags).toContainEqual(["name", "첫 번째 프로젝트"]);
  expect(announcement?.tags.filter((tag) => tag[0] === "clone")).toHaveLength(
    0,
  );
  const discussionTags =
    announcement?.tags.filter((tag) => tag[0] === "buzz-channel") ?? [];
  expect(discussionTags).toHaveLength(1);
  expect(discussionTags[0]?.[1]).toMatch(
    /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i,
  );

  await page
    .getByRole("button", { name: "View 첫 번째 프로젝트", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Open Discussion", exact: true }),
  ).toBeVisible();
});

test("top-level project lists align dates and overflow actions", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await page.addInitScript(() => {
    window.localStorage.setItem("buzz.projects.viewMode", "list");
  });
  await installMockBridge(page);
  await openMockApp(page);
  await page.getByTestId("open-projects-view").click();
  await expect(
    page.getByRole("heading", { level: 1, name: "Projects" }),
  ).toBeVisible();

  async function trailingPositions(row: import("@playwright/test").Locator) {
    await waitForAnimations(page);
    const date = row.getByTestId("projects-row-date");
    const menu = row.getByRole("button", { name: /More options for/ });
    await expect(date).toBeVisible();
    await expect(menu).toBeVisible();
    const dateBox = await date.boundingBox();
    const menuBox = await menu.boundingBox();
    expect(dateBox).not.toBeNull();
    expect(menuBox).not.toBeNull();
    return { dateX: dateBox?.x ?? 0, menuX: menuBox?.x ?? 0 };
  }

  await page.getByRole("button", { name: "Repositories", exact: true }).click();
  await page.getByRole("button", { name: "Filter repositories" }).click();
  await expect(
    page.getByRole("menuitem", { name: "My Repositories" }),
  ).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "Local" })).toBeVisible();
  await page.keyboard.press("Escape");
  const repositoryPositions = await trailingPositions(
    page.locator('[data-testid^="project-row-"]').first(),
  );
  await expect(
    page
      .locator('[data-testid^="project-row-"]')
      .first()
      .getByRole("button", { name: /Open .+ in SchoolX Code/ }),
  ).toBeVisible();

  await page
    .getByRole("button", { name: "Pull Requests", exact: true })
    .click();
  await page.getByRole("button", { name: "Filter pull requests" }).click();
  await expect(
    page.getByRole("menuitem", { name: "My Pull Requests" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await page.getByTestId("projects-create-menu").hover();
  await expect(
    page.getByRole("menuitem", { name: "Repository" }),
  ).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "Issue" })).toBeVisible();
  await page
    .getByRole("menuitem", { name: "Pull Request", exact: true })
    .click();
  await expect(page.getByTestId("create-pull-request-dialog")).toBeVisible();
  await expect(
    page.getByTestId("create-pull-request-repository"),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await page.getByTestId("projects-create-menu").hover();
  await page.getByRole("menuitem", { name: "Issue" }).click();
  await expect(page.getByTestId("create-issue-repository")).toBeVisible();
  await page.keyboard.press("Escape");
  const pullRequestRow = page
    .locator('[data-testid^="projects-pr-row-"]')
    .first();
  const pullRequestPositions = await trailingPositions(pullRequestRow);
  await pullRequestRow
    .getByRole("button", { name: /More options for/ })
    .click();
  await expect(
    page.getByRole("menuitem", { name: /Review PR|View (draft|merge|closed)/ }),
  ).toBeVisible();
  await page.keyboard.press("Escape");

  await page.getByRole("button", { name: "Issues", exact: true }).click();
  await page.getByRole("button", { name: "Filter issues" }).click();
  await expect(page.getByRole("menuitem", { name: "My Issues" })).toBeVisible();
  await page.keyboard.press("Escape");
  const issueRow = page.locator('[data-testid^="projects-issue-row-"]').first();
  const issuePositions = await trailingPositions(issueRow);

  expect(
    Math.abs(pullRequestPositions.dateX - repositoryPositions.dateX),
  ).toBeLessThanOrEqual(ALIGNMENT_TOLERANCE_PX);
  expect(
    Math.abs(pullRequestPositions.menuX - repositoryPositions.menuX),
  ).toBeLessThanOrEqual(ALIGNMENT_TOLERANCE_PX);
  expect(
    Math.abs(issuePositions.dateX - repositoryPositions.dateX),
  ).toBeLessThanOrEqual(ALIGNMENT_TOLERANCE_PX);
  expect(
    Math.abs(issuePositions.menuX - repositoryPositions.menuX),
  ).toBeLessThanOrEqual(ALIGNMENT_TOLERANCE_PX);

  await page.setViewportSize({ height: 720, width: 900 });
  await page.getByRole("button", { name: "Repositories", exact: true }).click();
  const responsiveRepositoryRow = page
    .locator('[data-testid^="project-row-"]')
    .first();
  await expect(
    responsiveRepositoryRow.getByTestId("projects-row-summary"),
  ).toBeHidden();
  await expect(
    responsiveRepositoryRow.getByTestId("projects-row-people"),
  ).toBeHidden();
  await expect(
    responsiveRepositoryRow.getByTestId("projects-row-date"),
  ).toBeVisible();
  await expect(
    responsiveRepositoryRow.getByRole("button", { name: /More options for/ }),
  ).toBeVisible();
  await expect(
    responsiveRepositoryRow.getByRole("button", {
      name: /Open .+ in SchoolX Code/,
    }),
  ).toBeVisible();
  expect(
    await responsiveRepositoryRow.evaluate(
      (row) => row.scrollWidth <= row.clientWidth,
    ),
  ).toBe(true);
});

test("commit detail opens from the commits feed with a diff", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  // The preview server is a static file server without SPA fallback, so
  // enter at "/" and navigate via the sidebar.
  await openMockApp(page);
  await page.getByTestId("open-projects-view").click();

  // The overview no longer lists repository cards — switch to the
  // Repositories filter to reveal the project cards/rows.
  await page.getByRole("button", { name: "Repositories", exact: true }).click();

  // Open the first mock project (dtag "buzz" from the e2e bridge fixture).
  const projectEntry = page
    .locator(
      '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
    )
    .first();
  await expect(projectEntry).toBeVisible({ timeout: 10_000 });
  await projectEntry
    .getByRole("button", { name: "View buzz", exact: true })
    .click();

  await page.getByRole("tab", { name: "Commits" }).click();
  const commitRows = page.getByTestId("project-activity-feed-item");
  await expect(commitRows.first()).toBeVisible({ timeout: 10_000 });

  // Commits share the rounded list structure used by issues and pull requests.
  await expect(
    page.getByRole("heading", { name: "Commits", exact: true }),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({
    fullPage: false,
    path: `${SHOTS}/02-commits-feed.png`,
  });

  // Open the newest commit via its subject button.
  await commitRows
    .first()
    .getByRole("button", { name: /Add Trello board workflow details/ })
    .click();

  // Detail header: author line, subject, and hash.
  await expect(page.getByText("Commit from")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Add Trello board workflow details" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Copy commit hash" }),
  ).toBeVisible();
  await expect(
    page.getByRole("link", { name: "project guide" }),
  ).toHaveAttribute("href", "https://example.com/project-guide");
  await expect(
    page.getByRole("button", { name: "Architecture" }),
  ).toBeVisible();
  await expect(page.locator("video")).toHaveAttribute(
    "src",
    "https://example.com/project-demo.mp4",
  );

  // Diff from the mocked get_project_repo_diff renders changed files.
  await expect(page.getByText("2 changed files")).toBeVisible({
    timeout: 10_000,
  });
  await expect(
    page.getByText("CommunityTabs({ selectedCommitHash })"),
  ).toBeVisible();

  await waitForAnimations(page);
  await page.screenshot({
    fullPage: false,
    path: `${SHOTS}/01-commit-detail.png`,
  });

  // Breadcrumb category segment steps back to the commits feed.
  await page
    .getByRole("navigation", { name: "Project breadcrumb" })
    .getByRole("button", { name: "Commits", exact: true })
    .click();
  await expect(commitRows.first()).toBeVisible();

  // The commits feed itself gets a grayed sub-tab crumb.
  await expect(
    page.getByRole("navigation", { name: "Project breadcrumb" }),
  ).toContainText("Commits");

  // The project-name segment goes to the project home (Overview tab).
  await commitRows
    .first()
    .getByRole("button", { name: /Add Trello board workflow details/ })
    .click();
  await expect(page.getByText("Commit from")).toBeVisible();
  await page
    .getByRole("navigation", { name: "Project breadcrumb" })
    .getByRole("button", { name: "buzz", exact: true })
    .click();
  await expect(page.getByRole("tab", { name: "Overview" })).toHaveAttribute(
    "aria-selected",
    "true",
  );

  // The Projects root segment leaves the project entirely.
  await page
    .getByRole("navigation", { name: "Project breadcrumb" })
    .getByRole("button", { name: "Projects", exact: true })
    .click();
  await expect(projectEntry).toBeVisible();
});

test("pull request and issue feeds share the commit row structure", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await openMockApp(page);
  await page.getByTestId("open-projects-view").click();

  // The overview no longer lists repository cards — switch to the
  // Repositories filter to reveal the project cards/rows.
  await page.getByRole("button", { name: "Repositories", exact: true }).click();

  const projectEntry = page
    .locator(
      '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
    )
    .first();
  await expect(projectEntry).toBeVisible({ timeout: 10_000 });
  await projectEntry
    .getByRole("button", { name: "View buzz", exact: true })
    .click();

  // PR rows use the shared feed row: title button + #id cluster cell.
  await page.getByRole("tab", { name: "Pull Request" }).click();
  const prRows = page.getByTestId("project-pull-request-row");
  await expect(prRows.first()).toBeVisible({ timeout: 10_000 });
  await expect(
    prRows.first().getByRole("button", { name: /^#/ }),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ fullPage: false, path: `${SHOTS}/03-prs-feed.png` });

  // The #id cell opens the PR detail, same as clicking the title.
  await prRows.first().getByRole("button", { name: /^#/ }).click();
  await expect(
    page.getByRole("navigation", { name: "Project breadcrumb" }),
  ).toContainText("Pull Request");

  // Step back to the feed so the community tabs are available again.
  await page
    .getByRole("navigation", { name: "Project breadcrumb" })
    .getByRole("button", { name: "Pull Request", exact: true })
    .click();
  await expect(prRows.first()).toBeVisible();

  // Issue rows share the same structure.
  await page.getByRole("tab", { name: "Issues" }).click();
  const issueRows = page.getByTestId("project-issue-row");
  await expect(issueRows.first()).toBeVisible({ timeout: 10_000 });
  await expect(
    issueRows.first().getByRole("button", { name: /^#/ }),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({
    fullPage: false,
    path: `${SHOTS}/04-issues-feed.png`,
  });
});
