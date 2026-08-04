import { expect, test } from "@playwright/test";
import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

/**
 * Render evidence for the workspace-catalog settings card.
 *
 * The card distinguishes 4 `outcome`s, 3 `user_action`s, 8 `decision`s, and the
 * `skipped`/`unrecognized` canvas states — but session D shipped it without a
 * single test that renders it, which is why Phase 3's completion criterion #7
 * stayed partially met (`docs/schoolx-2/IMPLEMENTATION_HANDOFF.md`, session D
 * "넘긴 것" #1). The machine-readable half of that criterion was already
 * covered by `ledger_serializes_for_ui_and_cli`; this is the UI half.
 *
 * Each test is a different render branch — item list, apply result, the two
 * recreate verdicts, gate refusal — so one breaking leaves the others green.
 * The two recreate tests are a pair: `deleted` has one sensible answer, while
 * `not_owned` cannot tell a squatter from a co-administrator and so must state
 * the consequence before offering the same control (`CATALOG_RECREATE.md` §4).
 * Their difference is the assertion, not either one alone.
 *
 * Navigation goes through `openSettings` rather than a direct URL on purpose.
 * The section is role-gated in `SettingsView`'s `visibleSections`, so walking
 * the profile menu → settings → nav item means a gate that wrongly hides the
 * section fails here instead of silently passing.
 */

const PREFLIGHT_ITEMS = [
  {
    item_key: "meeting",
    name: "메인 회의방",
    decision: "create_or_recreate",
    channel_id: null,
    channel_present: false,
    generation: 1,
    steps: { channel: "pending", canvas: "pending", membership: "pending" },
    renamed: false,
  },
  {
    item_key: "planning",
    name: "기획",
    decision: "adopted",
    channel_id: "11111111-2222-4333-8444-555555555555",
    channel_present: true,
    generation: 1,
    steps: { channel: "done", canvas: "done", membership: "done" },
    renamed: true,
  },
];

test("renders one row per catalog item, flagging the renamed one", async ({
  page,
}) => {
  await installMockBridge(page, {
    workspaceCatalogPreflight: PREFLIGHT_ITEMS,
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await openSettings(page, "workspace-catalog");

  await expect(page.getByTestId("settings-workspace-catalog")).toBeVisible();

  await expect(page.getByTestId("catalog-item-meeting")).toBeVisible();
  const planning = page.getByTestId("catalog-item-planning");
  await expect(planning).toBeVisible();
  // `name` is always the catalog display name, never the room's current one
  // (WORKSPACE_CATALOG.md §10) — a renamed room says so with a badge, not by
  // changing the name shown here.
  await expect(planning).toContainText("기획");
  await expect(page.getByTestId("catalog-renamed-planning")).toBeVisible();
  // And the unrenamed row carries no badge. Without this, a card that always
  // renders the badge would pass the assertion above.
  await expect(page.getByTestId("catalog-renamed-meeting")).toHaveCount(0);

  await waitForAnimations(page);
});

test("apply paints outcome, user action, and canvas notes from the ledger", async ({
  page,
}) => {
  await installMockBridge(page, {
    workspaceCatalogPreflight: PREFLIGHT_ITEMS,
    workspaceCatalogLedger: {
      catalog_id: "schoolx.default",
      catalog_version: 1,
      items: [
        {
          item_key: "meeting",
          name: "메인 회의방",
          decision: "create_or_recreate",
          channel_id: "22222222-3333-4444-8555-666666666666",
          generation: 1,
          steps: { channel: "done", canvas: "skipped", membership: "done" },
          outcome: "applied",
          user_action: null,
          renamed: false,
          error: null,
        },
        {
          item_key: "planning",
          name: "기획",
          decision: "not_owned",
          channel_id: "11111111-2222-4333-8444-555555555555",
          generation: 1,
          steps: { channel: "done", canvas: "pending", membership: "pending" },
          outcome: "blocked",
          user_action: "request_ownership",
          renamed: true,
          error: "이 방은 다른 사람이 만들었습니다",
        },
      ],
    },
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await openSettings(page, "workspace-catalog");

  await page.locator("#workspace-catalog-item-meeting").click();
  await page.getByTestId("catalog-apply").click();

  // A canvas the saga declined to overwrite is not the same as one it wrote;
  // the card has to say which happened.
  await expect(page.getByTestId("catalog-canvas-note-meeting")).toBeVisible();
  // A blocked item states what the user has to do about it.
  await expect(page.getByTestId("catalog-user-action-planning")).toBeVisible();
  await expect(page.getByTestId("catalog-error-planning")).toContainText(
    "이 방은 다른 사람이 만들었습니다",
  );

  await waitForAnimations(page);
});

/** A ledger with `meeting` blocked on the given verdict, everything else same. */
function blockedLedger(
  decision: "deleted" | "not_owned",
  user_action: "confirm_recreate" | "request_ownership",
) {
  return {
    catalog_id: "schoolx.default",
    catalog_version: 1,
    items: [
      {
        item_key: "meeting",
        name: "메인 회의방",
        decision,
        channel_id: "33333333-4444-4555-8666-777777777777",
        generation: 1,
        steps: {
          channel: decision === "not_owned" ? "done" : "pending",
          canvas: "pending",
          membership: "pending",
        },
        outcome: "blocked",
        user_action,
        renamed: false,
        error: null,
      },
    ],
  };
}

test("a deleted item offers the recreate control as its answer", async ({
  page,
}) => {
  await installMockBridge(page, {
    workspaceCatalogPreflight: PREFLIGHT_ITEMS,
    workspaceCatalogLedger: blockedLedger("deleted", "confirm_recreate"),
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await openSettings(page, "workspace-catalog");

  await page.locator("#workspace-catalog-item-meeting").click();
  await page.getByTestId("catalog-apply").click();

  const alert = page.getByTestId("catalog-user-action-meeting");
  await expect(alert).toBeVisible();
  await expect(page.getByTestId("catalog-recreate-meeting")).toBeVisible();
  // `deleted` carries no co-management risk — the room is gone, so there is
  // nothing a second one could duplicate. The warning belongs only to
  // `not_owned` and must not leak here.
  await expect(
    page.getByTestId("catalog-recreate-warning-meeting"),
  ).toHaveCount(0);

  await waitForAnimations(page);
});

test("not_owned warns that recreating makes a second room", async ({
  page,
}) => {
  await installMockBridge(page, {
    workspaceCatalogPreflight: PREFLIGHT_ITEMS,
    workspaceCatalogLedger: blockedLedger("not_owned", "request_ownership"),
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await openSettings(page, "workspace-catalog");

  await page.locator("#workspace-catalog-item-meeting").click();
  await page.getByTestId("catalog-apply").click();

  const alert = page.getByTestId("catalog-user-action-meeting");
  await expect(alert).toBeVisible();
  // The first line stays "ask whoever created it". Recreating is the way out
  // when that person is a squatter, and the consequence is stated before the
  // control rather than after (`CATALOG_RECREATE.md` §4).
  //
  // Asserted by test id, not by copy: the harness renders in English, and
  // pinning either translation would make a copy edit look like a regression.
  await expect(
    page.getByTestId("catalog-recreate-warning-meeting"),
  ).toBeVisible();
  await expect(page.getByTestId("catalog-recreate-meeting")).toBeVisible();

  await waitForAnimations(page);
});

test("a gate refusal explains itself and hides the apply button", async ({
  page,
}) => {
  await installMockBridge(page, {
    workspaceCatalogPreflightError: "catalog-admin-required",
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await openSettings(page, "workspace-catalog");

  await expect(page.getByTestId("catalog-admin-required")).toBeVisible();
  // A disabled button reads as "not yet". Nothing the user does on this screen
  // will enable it, so it is hidden rather than disabled.
  await expect(page.getByTestId("catalog-apply")).toHaveCount(0);

  await waitForAnimations(page);
});
