import { expect, test } from "@playwright/test";
import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

/**
 * The self-hosted relay entry points.
 *
 * What these pin is not connection logic — submitting a bare relay URL worked
 * before this feature existed (`InviteRedeemForm`'s `canSubmit` is true on
 * `normalizedRelayUrl` alone). What was missing was a *door*: every labelled
 * route either opened hosted sign-in or asked for an invite code, and an
 * invite can only be minted by an existing owner or admin, so a brand-new
 * relay has nobody to ask. See `docs/schoolx-2/SELF_HOSTED_ONBOARDING.md` §2.
 *
 * So the assertions are "is it reachable", one per door. The second door
 * matters most: it sits on the hosted dialog, which is where someone who
 * missed the first door actually gets stuck.
 *
 * `skipCommunitySeed` is what puts us on the onboarding screen at all —
 * `App.tsx` renders `WelcomeSetup` only while `community.needsSetup`.
 */

test("the existing-community page offers a self-hosted door", async ({
  page,
}) => {
  await installMockBridge(page, {}, { skipCommunitySeed: true });
  await page.goto("/", { waitUntil: "domcontentloaded" });

  await page.getByTestId("community-choice-existing").click();

  const card = page.getByTestId("self-hosted-relay-card");
  await expect(card).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({
    path: "test-results/self-hosted-onboarding/01-existing-page.png",
  });

  await card.click();
  await expect(page.getByTestId("self-hosted-relay-dialog")).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({
    path: "test-results/self-hosted-onboarding/02-relay-dialog.png",
  });
});

test("the hosted dialog has a way out to a self-hosted relay", async ({
  page,
}) => {
  await installMockBridge(page, {}, { skipCommunitySeed: true });
  await page.goto("/", { waitUntil: "domcontentloaded" });

  // "Create a community" is the door a school admin reaches for first, and it
  // opens hosted sign-in. This is the dead end reported from a real run.
  await page.getByTestId("community-choice-create").click();

  const link = page.getByTestId("self-hosted-relay-link");
  await expect(link).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({
    path: "test-results/self-hosted-onboarding/03-hosted-dialog-escape.png",
  });

  await link.click();
  await expect(page.getByTestId("self-hosted-relay-dialog")).toBeVisible();

  await waitForAnimations(page);
});
