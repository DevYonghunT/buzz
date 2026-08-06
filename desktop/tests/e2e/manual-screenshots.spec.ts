import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { seedActiveIdentity } from "../helpers/onboarding";
import { openSettings } from "../helpers/settings";

/**
 * Screenshots for the Korean first-run manual handed to teammates alongside a
 * team build (docs/schoolx-2/TEAM_BUILD.md). Each shot is one figure in
 * docs/schoolx-2/manual/USER_MANUAL.md — renumbering here means renumbering
 * there.
 *
 * Scoped shots use `locator.screenshot()` so two figures of the same view can
 * never come out byte-identical; the manual build gate hashes every PNG and
 * refuses duplicates.
 */

const SHOTS = "test-results/manual";

const BLANK_TYLER = { ...TEST_IDENTITIES.tyler, username: "" };

/**
 * Runtime catalog for the manual: Buzz Agent ready (what a teammate gets with
 * no extra install), Claude Code and Goose present so the harness surface is
 * shown as it really looks rather than as a one-row special case.
 */
const CATALOG = [
  {
    id: "buzz-agent",
    label: "Buzz Agent",
    avatar_url: "",
    availability: "available",
    command: "buzz-agent",
    binary_path: "/usr/local/bin/buzz-agent",
    default_args: [],
    mcp_command: "buzz-dev-mcp",
    install_hint: "",
    install_instructions_url: "https://github.com/block/buzz",
    can_auto_install: false,
    underlying_cli_path: null,
    node_required: false,
    auth_status: { status: "not_applicable" },
  },
  {
    id: "goose",
    label: "Goose",
    avatar_url: "",
    availability: "available",
    command: "goose",
    binary_path: "/usr/local/bin/goose",
    default_args: [],
    mcp_command: null,
    install_hint: "",
    install_instructions_url: "https://github.com/block/goose",
    can_auto_install: true,
    underlying_cli_path: "/usr/local/bin/goose",
    node_required: false,
    auth_status: { status: "not_applicable" },
  },
  {
    id: "claude",
    label: "Claude Code",
    avatar_url: "",
    availability: "available",
    command: "claude-agent-acp",
    binary_path: "/usr/local/bin/claude-agent-acp",
    default_args: [],
    mcp_command: null,
    install_hint: "",
    install_instructions_url:
      "https://github.com/agentclientprotocol/claude-agent-acp",
    can_auto_install: true,
    underlying_cli_path: "/usr/local/bin/claude",
    node_required: false,
    auth_status: { status: "logged_in" },
  },
];

test.use({ viewport: { width: 1280, height: 800 } });

test("manual: key creation and backup", async ({ page }) => {
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  // 01 — the very first screen: create a key, or bring an existing one.
  await expect(page.getByTestId("machine-onboarding-gate")).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/01-landing.png` });

  await page.getByRole("button", { name: "Create a new identity key" }).click();
  await expect(
    page.getByRole("heading", {
      name: "Your unique identity key has been created",
    }),
  ).toBeVisible();
  await waitForAnimations(page);
  // 02 — key created, still masked. This is the state a teammate lands in.
  await page.screenshot({ path: `${SHOTS}/02-key-created.png` });

  // 03 — revealed, because the manual has to show what "your key" looks like
  // before telling anyone to store it.
  await page.getByTestId("backup-key-reveal-toggle").click();
  await expect(page.getByTestId("backup-key-value")).toHaveClass(/select-text/);
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/03-key-revealed.png` });

  // 04 — the three ways to keep the key.
  await page.getByTestId("backup-options-link").click();
  await expect(
    page.getByTestId("onboarding-page-backup-options"),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/04-backup-options.png` });

  // 05 — the recommended one: a password-protected backup file.
  await page.getByTestId("backup-option-password").click();
  await expect(page.getByTestId("onboarding-page-download")).toBeVisible();
  await expect(page.getByTestId("backup-password-panel")).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/05-backup-password.png` });

  // 06 — harness (runtime) selection, reached by continuing past backup.
  await page.getByTestId("backup-return-to-onboarding").click();
  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();
  await page.getByTestId("onboarding-next").click();
  await expect(
    page.getByRole("heading", { name: "Set up your agent harnesses" }),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/06-harness-setup.png` });
});

test("manual: profile and avatar", async ({ page }) => {
  await seedActiveIdentity(page, BLANK_TYLER);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  // 07 — display name.
  await expect(page.getByTestId("onboarding-page-1")).toBeVisible();
  await page.getByTestId("onboarding-display-name").fill("김선생");
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/07-profile.png` });

  // 08 — avatar.
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/08-avatar.png` });
});

test("manual: main window", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });

  // 09 — the app as it opens: sidebar plus channel view.
  await expect(page.getByTestId("open-settings")).toBeVisible({
    timeout: 15_000,
  });
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/09-home.png` });
});

test("manual: harness settings", async ({ page }) => {
  test.setTimeout(60_000);
  // Tall viewport: the Harnesses panel sits below Agent defaults on the agents
  // settings page, and a scoped screenshot still needs the node laid out.
  await page.setViewportSize({ width: 1280, height: 2400 });
  await installMockBridge(page, { acpRuntimesCatalog: CATALOG });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await openSettings(page, "agents");

  // 10 — the Harnesses panel: which runtimes this machine has.
  const harnesses = page.getByTestId("settings-harnesses");
  await expect(harnesses).toBeVisible({ timeout: 15_000 });
  await page.waitForTimeout(700);
  await waitForAnimations(page);
  await harnesses.screenshot({ path: `${SHOTS}/10-settings-harnesses.png` });

  // 11 — the add-runtimes catalog behind the Add button.
  await page.getByTestId("harness-add-button").click();
  const dialog = page.getByTestId("harness-catalog-dialog");
  await expect(dialog).toBeVisible();
  await waitForAnimations(page);
  await dialog.screenshot({ path: `${SHOTS}/11-harness-catalog.png` });
});

test("manual: model and API key settings", async ({ page }) => {
  test.setTimeout(60_000);
  await installMockBridge(page, { acpRuntimesCatalog: CATALOG });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await openSettings(page, "agents");

  // 12 — the global agent defaults card: harness + model for every new agent.
  const card = page.getByTestId("settings-global-agent-config");
  await expect(card).toBeVisible({ timeout: 15_000 });
  await waitForAnimations(page);
  await card.screenshot({ path: `${SHOTS}/12-agent-defaults.png` });

  // 13 — the harness dropdown open, so the manual can point at "Buzz Agent".
  await page.getByTestId("global-agent-default-harness").click();
  await expect(
    page.getByTestId("global-agent-default-harness-option-buzz-agent"),
  ).toBeVisible();
  await waitForAnimations(page);
  // Full-page clip: an open dropdown is a portal, outside the card's box.
  await page.screenshot({ path: `${SHOTS}/13-harness-dropdown.png` });
  await page.keyboard.press("Escape");

  // 14 — Provider. Buzz Agent ships no LLM of its own, so Model stays disabled
  // until a provider is chosen; that ordering is the whole point of the manual
  // section and the figure has to show it.
  await page.getByTestId("global-agent-provider").click();
  await expect(
    page.getByTestId("global-agent-provider-option-anthropic"),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/14-provider-dropdown.png` });
  await page.getByTestId("global-agent-provider-option-anthropic").click();

  // 15 — the model dropdown, now enabled because a provider is set.
  const model = page.getByTestId("global-agent-model");
  await expect(model).toBeEnabled();
  await model.click();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/15-model-dropdown.png` });
  await page.keyboard.press("Escape");

  // 16 — the completed card. Choosing Anthropic reveals a required "Anthropic
  // API Key" field inline (not under Advanced); Advanced is expanded here only
  // so the figure shows the whole card at once. This is the step that decides
  // whether an agent runs at all, so it gets its own figure.
  await page.getByTestId("global-agent-advanced-toggle").click();
  await waitForAnimations(page);
  await card.screenshot({ path: `${SHOTS}/16-api-key.png` });
});
