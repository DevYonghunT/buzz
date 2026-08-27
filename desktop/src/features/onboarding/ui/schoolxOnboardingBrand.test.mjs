import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { en } from "@/shared/i18n/locales/en";
import { ko } from "@/shared/i18n/locales/ko";
import { SchoolXOnboardingBrand } from "./SchoolXOnboardingBrand.tsx";

const testDirectory = dirname(fileURLToPath(import.meta.url));
const desktopSource = resolve(testDirectory, "../../..");

test("SchoolX onboarding lockups use the canonical local mark", () => {
  const hero = renderToStaticMarkup(
    React.createElement(SchoolXOnboardingBrand, {
      productName: "SchoolX",
      variant: "hero",
    }),
  );
  assert.match(hero, /<h1/);
  assert.match(hero, /text-balance/);
  assert.match(hero, /src="\/brand\/schoolx-mark\.svg"/);
  assert.match(hero, /alt=""/);
  assert.match(hero, /aria-hidden="true"/);
  assert.match(hero, />SchoolX<\/span>/);

  const compact = renderToStaticMarkup(
    React.createElement(SchoolXOnboardingBrand, {
      productName: "스쿨엑스",
    }),
  );
  assert.match(compact, /<div/);
  assert.match(compact, />스쿨엑스<\/span>/);
});

test("first-run product surfaces do not restore Buzz presentation assets", () => {
  const productSurfaces = [
    join(desktopSource, "features/agents/ui/agentConfigControls.tsx"),
    join(testDirectory, "BackupStep.tsx"),
    join(testDirectory, "MachineOnboardingFlow.tsx"),
    join(testDirectory, "OnboardingChrome.tsx"),
    join(testDirectory, "PendingInviteGate.tsx"),
    join(testDirectory, "SetupStep.tsx"),
    join(
      desktopSource,
      "features/communities/ui/HostedCommunityCreateFlow.tsx",
    ),
    join(
      desktopSource,
      "features/communities/ui/HostedCommunityOnboarding.tsx",
    ),
    join(desktopSource, "features/communities/hostedCommunityApi.ts"),
  ];

  for (const filename of productSurfaces) {
    const source = readFileSync(filename, "utf8");
    assert.doesNotMatch(source, /\bBuzz\b/);
    assert.doesNotMatch(source, /shared\/ui\/buzz-logo/);
    assert.doesNotMatch(source, /buzz-wordmark\.png/);
    assert.doesNotMatch(source, /buzz-welcome-chartreuse/);
    assert.doesNotMatch(source, /LandingBees/);
  }

  const styles = readFileSync(
    join(desktopSource, "shared/styles/globals/components.css"),
    "utf8",
  );
  const shellStyles = styles.slice(
    styles.indexOf(".buzz-onboarding-neutral-theme.buzz-startup-shell"),
    styles.indexOf(".buzz-onboarding-key-text"),
  );
  assert.ok(shellStyles.length > 0);
  assert.doesNotMatch(shellStyles, /gradient\(/);
});

test("community profile and starter-team copy are localized as SchoolX", () => {
  assert.equal(
    en.app.onboarding.communityProfile.title,
    "Set up your SchoolX profile",
  );
  assert.equal(
    ko.app.onboarding.communityProfile.title,
    "SchoolX 프로필 만들기",
  );
  assert.match(en.app.onboarding.starterTeam.description, /SchoolX/);
  assert.match(ko.app.onboarding.starterTeam.description, /SchoolX/);
});
