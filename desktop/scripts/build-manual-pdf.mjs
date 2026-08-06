#!/usr/bin/env node
/**
 * Render docs/schoolx-2/manual/manual.html to a PDF for teammates receiving a
 * team build (docs/schoolx-2/TEAM_BUILD.md).
 *
 * Lives here rather than beside the manual because Playwright's Chromium is
 * installed in the desktop workspace; a script under docs/ cannot resolve it.
 *
 *   node scripts/build-manual-pdf.mjs        # from desktop/
 *
 * The figures come from tests/e2e/manual-screenshots.spec.ts. Refresh them
 * whenever the screens change, then rebuild:
 *
 *   pnpm build:e2e
 *   pnpm exec playwright test manual-screenshots --project=smoke
 *   cp test-results/manual/*.png ../docs/schoolx-2/manual/images/
 *   node scripts/build-manual-pdf.mjs
 */

import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "@playwright/test";

const here = dirname(fileURLToPath(import.meta.url));
const manualDir = join(here, "..", "..", "docs", "schoolx-2", "manual");
const source = join(manualDir, "manual.html");
const output = join(manualDir, "SchoolX-사용설명서.pdf");

if (!existsSync(source)) {
  console.error(`manual.html not found at ${source}`);
  process.exit(1);
}

const browser = await chromium.launch();
try {
  const page = await browser.newPage();
  // file:// so the relative images/ paths resolve without a server.
  await page.goto(`file://${source}`, { waitUntil: "networkidle" });

  // Every figure must be decoded before the PDF snapshot, or images come out
  // as blank boxes.
  await page.evaluate(() =>
    Promise.all(
      Array.from(document.images)
        .filter((img) => !img.complete)
        .map(
          (img) =>
            new Promise((resolve) => {
              img.addEventListener("load", resolve, { once: true });
              img.addEventListener("error", resolve, { once: true });
            }),
        ),
    ),
  );

  const broken = await page.evaluate(() =>
    Array.from(document.images)
      .filter((img) => img.naturalWidth === 0)
      .map((img) => img.getAttribute("src")),
  );
  if (broken.length > 0) {
    console.error(`Missing images:\n  ${broken.join("\n  ")}`);
    process.exit(1);
  }

  await page.pdf({
    path: output,
    format: "A4",
    printBackground: true,
    displayHeaderFooter: true,
    headerTemplate: "<div></div>",
    footerTemplate: `
      <div style="width:100%;font-size:8pt;color:#94a3b8;
                  font-family:-apple-system,sans-serif;
                  padding:0 16mm;display:flex;justify-content:space-between;">
        <span>SchoolX 사용 설명서</span>
        <span class="pageNumber"></span>
      </div>`,
  });

  console.log(output);
} finally {
  await browser.close();
}
