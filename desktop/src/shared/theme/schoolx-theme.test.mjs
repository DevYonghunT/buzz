import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";

import { hexToHsl } from "./adaptive-theme.ts";
import {
  SCHOOLX_PALETTE,
  SCHOOLX_THEME_CACHE_REVISION,
  canApplyThemeCache,
  createSchoolXTheme,
  getSchoolXThemeDisplayName,
  parseThemeCachePayload,
  resolveStoredFollowSystem,
  resolveStoredThemeName,
} from "./schoolx-theme.ts";
import { SYNTAX_THEMES, resolveShikiThemeName } from "./theme-loader.ts";

const INDEX_HTML_URL = new URL("../../../index.html", import.meta.url);
const THEME_CSS_URL = new URL("../styles/globals/theme.css", import.meta.url);

function rgb(hex) {
  return [1, 3, 5].map((offset) =>
    Number.parseInt(hex.slice(offset, offset + 2), 16),
  );
}

function relativeLuminanceChannels(channels) {
  const linearChannels = channels.map((channel) => {
    const value = channel / 255;
    return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return (
    0.2126 * linearChannels[0] +
    0.7152 * linearChannels[1] +
    0.0722 * linearChannels[2]
  );
}

function contrastChannels(a, b) {
  const aLuminance = relativeLuminanceChannels(a);
  const bLuminance = relativeLuminanceChannels(b);
  return (
    (Math.max(aLuminance, bLuminance) + 0.05) /
    (Math.min(aLuminance, bLuminance) + 0.05)
  );
}

function contrastRatio(a, b) {
  return contrastChannels(rgb(a), rgb(b));
}

function hslToRgb(value) {
  const match = value.match(/^([\d.]+) ([\d.]+)% ([\d.]+)%$/);
  assert.ok(match, `expected HSL components, got ${value}`);
  const hue = Number(match[1]) / 60;
  const saturation = Number(match[2]) / 100;
  const lightness = Number(match[3]) / 100;
  const chroma = (1 - Math.abs(2 * lightness - 1)) * saturation;
  const x = chroma * (1 - Math.abs((hue % 2) - 1));
  const [r, g, b] =
    hue < 1
      ? [chroma, x, 0]
      : hue < 2
        ? [x, chroma, 0]
        : hue < 3
          ? [0, chroma, x]
          : hue < 4
            ? [0, x, chroma]
            : hue < 5
              ? [x, 0, chroma]
              : [chroma, 0, x];
  const m = lightness - chroma / 2;
  return [r, g, b].map((channel) => (channel + m) * 255);
}

function compositeChannels(foreground, background, alpha) {
  return foreground.map(
    (channel, index) => channel * alpha + background[index] * (1 - alpha),
  );
}

function parseRgba(value) {
  const match = value.match(/^rgba\(([\d.]+), ([\d.]+), ([\d.]+), ([\d.]+)\)$/);
  assert.ok(match, `expected rgba(), got ${value}`);
  return {
    alpha: Number(match[4]),
    channels: [Number(match[1]), Number(match[2]), Number(match[3])],
  };
}

function compositeHex(foreground, background, alpha) {
  const foregroundChannels = rgb(foreground);
  const backgroundChannels = rgb(background);
  return `#${foregroundChannels
    .map((channel, index) =>
      Math.round(channel * alpha + backgroundChannels[index] * (1 - alpha))
        .toString(16)
        .padStart(2, "0"),
    )
    .join("")}`;
}

function cacheFor(themeName) {
  const theme = createSchoolXTheme(themeName);
  return {
    isDark: theme.isDark,
    revision: SCHOOLX_THEME_CACHE_REVISION,
    themeName,
    vars: theme.vars,
  };
}

async function readPrepaintScript() {
  const html = await readFile(INDEX_HTML_URL, "utf8");
  const match = html.match(/<script>\s*([\s\S]*?)<\/script>/);
  assert.ok(match, "index.html must contain the synchronous prepaint script");
  return { html, script: match[1] };
}

async function runPrepaint({
  cache = null,
  followsSystem = null,
  storedTheme = null,
  systemIsDark = false,
} = {}) {
  const { script } = await readPrepaintScript();
  const storage = new Map();
  if (cache !== null) storage.set("buzz-theme-cache", cache);
  if (followsSystem !== null) {
    storage.set("buzz-follow-system", followsSystem);
  }
  if (storedTheme !== null) storage.set("buzz-theme", storedTheme);

  const classes = new Set();
  const attributes = new Map();
  const variables = new Map();
  const style = {
    backgroundColor: "",
    setProperty(name, value) {
      variables.set(name, value);
    },
  };
  const documentElement = {
    classList: {
      add(...names) {
        for (const name of names) classes.add(name);
      },
      remove(...names) {
        for (const name of names) classes.delete(name);
      },
    },
    setAttribute(name, value) {
      attributes.set(name, value);
    },
    style,
  };

  vm.runInNewContext(script, {
    document: { documentElement },
    window: {
      localStorage: { getItem: (key) => storage.get(key) ?? null },
      matchMedia: () => ({ matches: systemIsDark }),
    },
  });
  return { attributes, classes, style, variables };
}

test("canonical SchoolX palette and fixed contrast decisions stay exact", () => {
  assert.deepEqual(SCHOOLX_PALETTE, {
    ink: "#1F2937",
    parchment: "#F4EDDD",
    pine: "#355649",
    sage: "#7F967A",
    terracotta: "#B85A3C",
    terracottaDark: "#D97958",
    warmGold: "#D7A94B",
  });

  const expectedRatios = [
    [SCHOOLX_PALETTE.ink, SCHOOLX_PALETTE.parchment, 12.59],
    [SCHOOLX_PALETTE.parchment, SCHOOLX_PALETTE.pine, 6.98],
    ["#FFFFFF", SCHOOLX_PALETTE.terracotta, 4.6],
    [SCHOOLX_PALETTE.ink, SCHOOLX_PALETTE.terracottaDark, 4.77],
    [SCHOOLX_PALETTE.ink, SCHOOLX_PALETTE.warmGold, 6.76],
    [SCHOOLX_PALETTE.ink, SCHOOLX_PALETTE.sage, 4.58],
    [SCHOOLX_PALETTE.parchment, SCHOOLX_PALETTE.terracotta, 3.94],
    [SCHOOLX_PALETTE.parchment, SCHOOLX_PALETTE.sage, 2.75],
    [SCHOOLX_PALETTE.pine, SCHOOLX_PALETTE.ink, 1.8],
  ];
  for (const [foreground, background, expected] of expectedRatios) {
    assert.equal(
      Number(contrastRatio(foreground, background).toFixed(2)),
      expected,
    );
  }
});

test("first-party aliases resolve complete SchoolX semantic maps", () => {
  const light = createSchoolXTheme("buzz");
  const dark = createSchoolXTheme("buzz-dark");

  assert.equal(light.isDark, false);
  assert.equal(light.vars["--background"], hexToHsl(SCHOOLX_PALETTE.parchment));
  assert.equal(light.vars["--foreground"], hexToHsl(SCHOOLX_PALETTE.ink));
  assert.equal(light.vars["--primary"], hexToHsl(SCHOOLX_PALETTE.pine));
  assert.equal(
    light.vars["--sidebar-background"],
    hexToHsl(SCHOOLX_PALETTE.pine),
  );
  assert.equal(light.vars["--sidebar-accent"], hexToHsl("#4E6A5C"));
  assert.equal(
    light.vars["--schoolx-action"],
    hexToHsl(SCHOOLX_PALETTE.terracotta),
  );
  assert.notEqual(light.vars["--destructive"], light.vars["--schoolx-action"]);

  assert.equal(dark.isDark, true);
  assert.equal(dark.vars["--background"], hexToHsl(SCHOOLX_PALETTE.ink));
  assert.equal(dark.vars["--foreground"], hexToHsl(SCHOOLX_PALETTE.parchment));
  assert.equal(
    dark.vars["--primary"],
    hexToHsl(SCHOOLX_PALETTE.terracottaDark),
  );
  assert.equal(
    dark.vars["--primary-foreground"],
    hexToHsl(SCHOOLX_PALETTE.ink),
  );
  assert.equal(
    dark.vars["--schoolx-nav-active-border"],
    hexToHsl(SCHOOLX_PALETTE.warmGold),
  );

  const requiredVars = [
    "--background",
    "--foreground",
    "--card",
    "--popover",
    "--primary",
    "--secondary",
    "--muted",
    "--accent",
    "--destructive",
    "--border",
    "--input",
    "--ring",
    "--sidebar-background",
    "--sidebar-foreground",
    "--sidebar-active",
    "--sidebar-ring",
    "--status-added",
    "--status-deleted",
    "--status-modified",
    "--ui-warning",
  ];
  for (const name of requiredVars) {
    assert.equal(typeof light.vars[name], "string", `${name} missing in light`);
    assert.equal(typeof dark.vars[name], "string", `${name} missing in dark`);
  }
});

test("small sidebar text clears contrast on bare and search surfaces", () => {
  const mutedOnPine = compositeHex(
    SCHOOLX_PALETTE.parchment,
    SCHOOLX_PALETTE.pine,
    0.76,
  );
  assert.ok(contrastRatio(mutedOnPine, SCHOOLX_PALETTE.pine) >= 4.5);
  assert.ok(contrastRatio(SCHOOLX_PALETTE.parchment, "#4E6A5C") >= 4.5);
});

test("complete semantic foreground pairs pass on their actual surfaces", () => {
  const directPairs = [
    ["--foreground", "--background"],
    ["--card-foreground", "--card"],
    ["--popover-foreground", "--popover"],
    ["--muted-foreground", "--muted"],
    ["--secondary-foreground", "--secondary"],
    ["--accent-foreground", "--accent"],
    ["--primary-foreground", "--primary"],
    ["--destructive-foreground", "--destructive"],
    ["--sidebar-foreground", "--sidebar-background"],
    ["--sidebar-accent-foreground", "--sidebar-accent"],
    ["--sidebar-primary-foreground", "--sidebar-primary"],
    ["--sidebar-active-foreground", "--sidebar-active"],
  ];
  const surfaces = [
    "--background",
    "--card",
    "--popover",
    "--muted",
    "--secondary",
    "--accent",
  ];

  for (const themeName of ["buzz", "buzz-dark"]) {
    const { vars } = createSchoolXTheme(themeName);
    for (const [foreground, background] of directPairs) {
      assert.ok(
        contrastChannels(
          hslToRgb(vars[foreground]),
          hslToRgb(vars[background]),
        ) >= 4.5,
        `${themeName} ${foreground} on ${background}`,
      );
    }

    for (const status of [
      "--status-added",
      "--status-deleted",
      "--status-modified",
    ]) {
      for (const surface of surfaces) {
        assert.ok(
          contrastChannels(rgb(vars[status]), hslToRgb(vars[surface])) >= 4.5,
          `${themeName} ${status} on ${surface}`,
        );
      }
    }

    const warningBackground = parseRgba(vars["--ui-warning-bg"]);
    for (const surface of surfaces) {
      const compositedBackground = compositeChannels(
        warningBackground.channels,
        hslToRgb(vars[surface]),
        warningBackground.alpha,
      );
      assert.ok(
        contrastChannels(rgb(vars["--ui-warning"]), compositedBackground) >=
          4.5,
        `${themeName} warning text on tinted ${surface}`,
      );
    }
  }
});

test("focus and selection boundaries retain non-text contrast", () => {
  for (const themeName of ["buzz", "buzz-dark"]) {
    const { vars } = createSchoolXTheme(themeName);
    assert.ok(
      contrastChannels(
        hslToRgb(vars["--ring"]),
        hslToRgb(vars["--background"]),
      ) >= 3,
    );
    assert.ok(
      Math.max(
        contrastChannels(
          hslToRgb(vars["--schoolx-nav-active"]),
          hslToRgb(vars["--sidebar-background"]),
        ),
        contrastChannels(
          hslToRgb(vars["--schoolx-nav-active-border"]),
          hslToRgb(vars["--sidebar-background"]),
        ),
      ) >= 3,
    );
  }
});

test("public labels change while IDs and Shiki mappings remain compatible", () => {
  assert.equal(getSchoolXThemeDisplayName("buzz"), "SchoolX");
  assert.equal(getSchoolXThemeDisplayName("buzz-dark"), "SchoolX Dark");
  assert.equal(getSchoolXThemeDisplayName("github-light"), null);
  assert.equal(resolveShikiThemeName("buzz"), "github-light");
  assert.equal(resolveShikiThemeName("buzz-dark"), "github-dark");
});

test("stored theme recovery preserves follow-system state", () => {
  const supported = (name) =>
    ["buzz", "buzz-dark", "github-light"].includes(name);
  assert.equal(resolveStoredThemeName(null, "buzz", supported), "buzz");
  assert.equal(
    resolveStoredThemeName("unsupported", "buzz", supported),
    "buzz",
  );
  assert.equal(
    resolveStoredThemeName("github-light", "buzz", supported),
    "github-light",
  );
  assert.equal(
    resolveStoredThemeName("light", "buzz", supported),
    "catppuccin-latte",
  );
  assert.equal(resolveStoredThemeName("dark", "buzz", supported), "houston");
  assert.equal(resolveStoredFollowSystem("true", "unsupported"), true);
  assert.equal(resolveStoredFollowSystem("false", null), false);
  assert.equal(resolveStoredFollowSystem(null, null), true);
  assert.equal(resolveStoredFollowSystem(null, "buzz"), false);
});

test("first-party cache accepts only the current complete effective palette", () => {
  const light = cacheFor("buzz");
  const dark = cacheFor("buzz-dark");
  assert.equal(canApplyThemeCache(light, "buzz"), true);
  assert.equal(canApplyThemeCache(dark, "buzz-dark"), true);
  assert.equal(canApplyThemeCache(light, "buzz-dark"), false);
  assert.equal(
    canApplyThemeCache({ ...light, revision: undefined }, "buzz"),
    false,
  );
  assert.equal(
    canApplyThemeCache({ ...light, revision: "old" }, "buzz"),
    false,
  );
  assert.equal(canApplyThemeCache({ ...light, vars: {} }, "buzz"), false);
  assert.equal(
    canApplyThemeCache(
      { ...light, vars: { "--background": light.vars["--background"] } },
      "buzz",
    ),
    false,
  );
  assert.equal(
    canApplyThemeCache(
      {
        ...light,
        vars: { ...light.vars, "--background": "sentinel" },
      },
      "buzz",
    ),
    false,
  );
  assert.equal(
    canApplyThemeCache(
      { isDark: false, themeName: "github-light", vars: {} },
      "github-light",
    ),
    true,
  );
});

test("cache parser rejects invalid JSON and invalid shapes", () => {
  assert.equal(parseThemeCachePayload("{"), null);
  assert.equal(parseThemeCachePayload("null"), null);
  assert.equal(
    parseThemeCachePayload(
      JSON.stringify({ isDark: false, themeName: "buzz", vars: [] }),
    ),
    null,
  );
  assert.equal(
    parseThemeCachePayload(
      JSON.stringify({
        isDark: false,
        themeName: "buzz",
        vars: { background: "sentinel" },
      }),
    ),
    null,
  );
  assert.deepEqual(
    parseThemeCachePayload(JSON.stringify(cacheFor("buzz"))),
    cacheFor("buzz"),
  );
});

test("index prepaint revision literal stays byte-for-byte aligned", async () => {
  const { html } = await readPrepaintScript();
  const revision = html.match(
    /SCHOOLX_THEME_CACHE_REVISION\s*=\s*"([^"]+)"/,
  )?.[1];
  assert.equal(revision, SCHOOLX_THEME_CACHE_REVISION);

  const supportedNamesSource = html.match(
    /var SUPPORTED_THEME_NAMES = new Set\((\[[\s\S]*?\])\);/,
  )?.[1];
  assert.ok(supportedNamesSource, "index.html must pin supported theme IDs");
  assert.deepEqual(Array.from(vm.runInNewContext(supportedNamesSource)), [
    ...SYNTAX_THEMES,
  ]);
});

test("fresh installs default to the SchoolX pair and follow the system", async () => {
  const light = await runPrepaint({ systemIsDark: false });
  assert.equal(light.classes.has("light"), true);
  assert.equal(light.attributes.get("data-buzz-theme"), "buzz");
  assert.equal(light.style.backgroundColor, "#f4eddd");

  const dark = await runPrepaint({ systemIsDark: true });
  assert.equal(dark.classes.has("dark"), true);
  assert.equal(dark.attributes.get("data-buzz-theme"), "buzz-dark");
  assert.equal(dark.style.backgroundColor, "#1f2937");
});

test("index prepaint rejects stale and incomplete first-party caches", async () => {
  const sentinelVars = {
    "--background": "300 100% 50%",
    "--foreground": "300 100% 50%",
  };
  for (const cache of [
    { isDark: false, themeName: "buzz", vars: sentinelVars },
    {
      isDark: false,
      revision: SCHOOLX_THEME_CACHE_REVISION,
      themeName: "buzz",
      vars: {},
    },
    {
      isDark: false,
      revision: SCHOOLX_THEME_CACHE_REVISION,
      themeName: "buzz",
      vars: sentinelVars,
    },
    {
      ...cacheFor("buzz"),
      vars: {
        ...cacheFor("buzz").vars,
        "--background": "300 100% 50%",
      },
    },
  ]) {
    const result = await runPrepaint({ cache: JSON.stringify(cache) });
    assert.equal(result.style.backgroundColor, "#f4eddd");
    assert.equal(
      result.variables.get("--background"),
      hexToHsl(SCHOOLX_PALETTE.parchment),
    );
    assert.equal(result.attributes.get("data-buzz-theme"), "buzz");
  }
});

test("index prepaint rejects a third-party cache beside an unsupported stored ID", async () => {
  const result = await runPrepaint({
    cache: JSON.stringify({
      isDark: false,
      themeName: "github-light",
      vars: { "--background": "0 0% 100%" },
    }),
    followsSystem: "true",
    storedTheme: "unsupported-theme",
    systemIsDark: true,
  });
  assert.equal(result.style.backgroundColor, "#1f2937");
  assert.equal(
    result.variables.get("--background"),
    hexToHsl(SCHOOLX_PALETTE.ink),
  );
  assert.equal(result.attributes.get("data-buzz-theme"), "buzz-dark");
});

test("index prepaint accepts a valid first-party round trip and rejects mode mismatch", async () => {
  const light = cacheFor("buzz");
  const valid = await runPrepaint({ cache: JSON.stringify(light) });
  assert.equal(valid.classes.has("light"), true);
  assert.equal(valid.variables.get("--background"), light.vars["--background"]);
  assert.equal(valid.attributes.get("data-buzz-theme"), "buzz");

  const mismatch = await runPrepaint({
    cache: JSON.stringify(light),
    systemIsDark: true,
  });
  assert.equal(mismatch.classes.has("dark"), true);
  assert.equal(mismatch.style.backgroundColor, "#1f2937");
  assert.equal(
    mismatch.variables.get("--background"),
    hexToHsl(SCHOOLX_PALETTE.ink),
  );
});

test("first-party shell CSS contains no legacy gradient or translucency rule", async () => {
  const css = await readFile(THEME_CSS_URL, "utf8");
  assert.doesNotMatch(css, /--buzz-gradient-/);
  assert.doesNotMatch(css, /:root\[data-buzz-translucent\]/);
  assert.doesNotMatch(css, /linear-gradient/);
  assert.match(css, /background-color: hsl\(var\(--sidebar-background\)\)/);
});
