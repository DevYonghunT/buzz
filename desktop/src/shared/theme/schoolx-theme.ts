import { hexToHsl, type ThemeResult } from "./adaptive-theme";

/** Revision required on cached first-party SchoolX palette payloads. */
export const SCHOOLX_THEME_CACHE_REVISION = "schoolx-v1";

/** Canonical SchoolX brand colors. Keep product components hex-free. */
export const SCHOOLX_PALETTE = {
  ink: "#1F2937",
  parchment: "#F4EDDD",
  pine: "#355649",
  sage: "#7F967A",
  terracotta: "#B85A3C",
  terracottaDark: "#D97958",
  warmGold: "#D7A94B",
} as const;

export type SchoolXThemeName = "buzz" | "buzz-dark";

export type ThemeCachePayload = {
  isDark: boolean;
  revision?: string;
  themeName: string;
  vars: Record<string, string>;
};

/** Inline variables that must not survive a switch to a third-party theme. */
export const SCHOOLX_THEME_ONLY_VAR_NAMES = [
  "--schoolx-action",
  "--schoolx-action-foreground",
  "--schoolx-highlight",
  "--schoolx-highlight-foreground",
  "--schoolx-nav-active",
  "--schoolx-nav-active-border",
  "--schoolx-nav-active-foreground",
  "--schoolx-success",
  "--schoolx-success-foreground",
] as const;

const WHITE = "#FFFFFF";
const LIGHT_DESTRUCTIVE = "#A8332A";
const DARK_DESTRUCTIVE = "#F08A80";

type Rgb = { b: number; g: number; r: number };

function hexToRgb(hex: string): Rgb {
  return {
    r: Number.parseInt(hex.slice(1, 3), 16),
    g: Number.parseInt(hex.slice(3, 5), 16),
    b: Number.parseInt(hex.slice(5, 7), 16),
  };
}

function mixHex(fromHex: string, toHex: string, factor: number): string {
  const from = hexToRgb(fromHex);
  const to = hexToRgb(toHex);
  const channel = (key: keyof Rgb) =>
    Math.round(from[key] + (to[key] - from[key]) * factor)
      .toString(16)
      .padStart(2, "0");
  return `#${channel("r")}${channel("g")}${channel("b")}`.toUpperCase();
}

function alpha(hex: string, opacity: number): string {
  const { b, g, r } = hexToRgb(hex);
  return `rgba(${r}, ${g}, ${b}, ${opacity})`;
}

function hslVars(colors: Record<string, string>): Record<string, string> {
  return Object.fromEntries(
    Object.entries(colors).map(([name, value]) => [name, hexToHsl(value)]),
  );
}

function commonStatusVars(isDark: boolean): Record<string, string> {
  const { ink, parchment, sage, terracotta, terracottaDark, warmGold } =
    SCHOOLX_PALETTE;
  const warning = isDark
    ? mixHex(terracottaDark, parchment, 0.65)
    : mixHex(terracotta, ink, 0.45);
  return {
    "--status-added": isDark
      ? mixHex(sage, parchment, 0.55)
      : mixHex(sage, ink, 0.55),
    "--status-deleted": isDark
      ? mixHex(terracottaDark, parchment, 0.55)
      : mixHex(terracotta, ink, 0.35),
    "--status-modified": isDark
      ? mixHex(warmGold, parchment, 0.3)
      : mixHex(warmGold, ink, 0.65),
    "--ui-warning": warning,
    "--ui-warning-bg": alpha(isDark ? terracottaDark : terracotta, 0.12),
  };
}

function createLightVars(): Record<string, string> {
  const { ink, parchment, pine, sage, terracotta, warmGold } = SCHOOLX_PALETTE;
  return {
    ...hslVars({
      "--accent": mixHex(parchment, warmGold, 0.3),
      "--accent-foreground": ink,
      "--background": parchment,
      "--border": mixHex(parchment, pine, 0.28),
      "--card": mixHex(parchment, WHITE, 0.32),
      "--card-foreground": ink,
      "--destructive": LIGHT_DESTRUCTIVE,
      "--destructive-foreground": WHITE,
      "--foreground": ink,
      "--huddle-control-chevron-hover-surface": mixHex(ink, WHITE, 0.18),
      "--huddle-control-chevron-surface": mixHex(ink, WHITE, 0.1),
      "--huddle-control-foreground": parchment,
      "--huddle-control-hover-surface": mixHex(ink, WHITE, 0.24),
      "--huddle-control-surface": mixHex(ink, WHITE, 0.16),
      "--huddle-drawer-surface": ink,
      "--huddle-popover-border": mixHex(ink, parchment, 0.24),
      "--huddle-popover-surface": mixHex(ink, WHITE, 0.1),
      "--huddle-tooltip-foreground": parchment,
      "--huddle-tooltip-surface": mixHex(ink, WHITE, 0.16),
      "--input": mixHex(parchment, pine, 0.28),
      "--muted": mixHex(parchment, pine, 0.1),
      "--muted-foreground": pine,
      "--popover": mixHex(parchment, WHITE, 0.58),
      "--popover-foreground": ink,
      "--primary": pine,
      "--primary-foreground": parchment,
      "--ring": terracotta,
      "--secondary": mixHex(parchment, sage, 0.3),
      "--secondary-foreground": ink,
      "--sidebar-accent": mixHex(pine, parchment, 0.13),
      "--sidebar-accent-foreground": parchment,
      "--sidebar-active": terracotta,
      "--sidebar-active-foreground": WHITE,
      "--sidebar-background": pine,
      "--sidebar-border": mixHex(pine, parchment, 0.34),
      "--sidebar-foreground": parchment,
      "--sidebar-primary": terracotta,
      "--sidebar-primary-foreground": WHITE,
      "--sidebar-ring": warmGold,
      "--schoolx-action": terracotta,
      "--schoolx-action-foreground": WHITE,
      "--schoolx-highlight": warmGold,
      "--schoolx-highlight-foreground": ink,
      "--schoolx-nav-active": parchment,
      "--schoolx-nav-active-border": warmGold,
      "--schoolx-nav-active-foreground": ink,
      "--schoolx-success": sage,
      "--schoolx-success-foreground": ink,
    }),
    ...commonStatusVars(false),
  };
}

function createDarkVars(): Record<string, string> {
  const { ink, parchment, pine, sage, terracottaDark, warmGold } =
    SCHOOLX_PALETTE;
  const sidebar = mixHex(ink, pine, 0.32);
  return {
    ...hslVars({
      "--accent": mixHex(ink, warmGold, 0.18),
      "--accent-foreground": parchment,
      "--background": ink,
      "--border": mixHex(ink, parchment, 0.22),
      "--card": mixHex(ink, pine, 0.18),
      "--card-foreground": parchment,
      "--destructive": DARK_DESTRUCTIVE,
      "--destructive-foreground": ink,
      "--foreground": parchment,
      "--huddle-control-chevron-hover-surface": mixHex(ink, parchment, 0.18),
      "--huddle-control-chevron-surface": mixHex(ink, parchment, 0.1),
      "--huddle-control-foreground": parchment,
      "--huddle-control-hover-surface": mixHex(ink, parchment, 0.24),
      "--huddle-control-surface": mixHex(ink, parchment, 0.16),
      "--huddle-drawer-surface": ink,
      "--huddle-popover-border": mixHex(ink, parchment, 0.24),
      "--huddle-popover-surface": mixHex(ink, parchment, 0.1),
      "--huddle-tooltip-foreground": parchment,
      "--huddle-tooltip-surface": mixHex(ink, parchment, 0.16),
      "--input": mixHex(ink, parchment, 0.22),
      "--muted": mixHex(ink, parchment, 0.09),
      "--muted-foreground": mixHex(sage, parchment, 0.35),
      "--popover": mixHex(ink, parchment, 0.1),
      "--popover-foreground": parchment,
      "--primary": terracottaDark,
      "--primary-foreground": ink,
      "--ring": warmGold,
      "--secondary": pine,
      "--secondary-foreground": parchment,
      "--sidebar-accent": mixHex(ink, pine, 0.6),
      "--sidebar-accent-foreground": parchment,
      "--sidebar-active": terracottaDark,
      "--sidebar-active-foreground": ink,
      "--sidebar-background": sidebar,
      "--sidebar-border": mixHex(ink, parchment, 0.25),
      "--sidebar-foreground": parchment,
      "--sidebar-primary": terracottaDark,
      "--sidebar-primary-foreground": ink,
      "--sidebar-ring": warmGold,
      "--schoolx-action": terracottaDark,
      "--schoolx-action-foreground": ink,
      "--schoolx-highlight": warmGold,
      "--schoolx-highlight-foreground": ink,
      "--schoolx-nav-active": pine,
      "--schoolx-nav-active-border": warmGold,
      "--schoolx-nav-active-foreground": parchment,
      "--schoolx-success": sage,
      "--schoolx-success-foreground": ink,
    }),
    ...commonStatusVars(true),
  };
}

const SCHOOLX_LIGHT_VARS = createLightVars();
const SCHOOLX_DARK_VARS = createDarkVars();

/** Whether an internal persisted theme ID selects a first-party SchoolX theme. */
export function isSchoolXThemeName(name: string): name is SchoolXThemeName {
  return name === "buzz" || name === "buzz-dark";
}

/** Public label for a first-party compatibility theme ID. */
export function getSchoolXThemeDisplayName(name: string): string | null {
  if (name === "buzz") return "SchoolX";
  if (name === "buzz-dark") return "SchoolX Dark";
  return null;
}

/** Resolve persisted legacy/invalid theme values without changing follow state. */
export function resolveStoredThemeName(
  storedTheme: string | null,
  fallback: string,
  isSupported: (name: string) => boolean,
): string {
  if (!storedTheme) return fallback;
  if (storedTheme === "light") return "catppuccin-latte";
  if (storedTheme === "dark" || storedTheme === "system") return "houston";
  return isSupported(storedTheme) ? storedTheme : fallback;
}

/** Preserve an explicit follow-system value; only fresh profiles default on. */
export function resolveStoredFollowSystem(
  storedFollowSystem: string | null,
  storedTheme: string | null,
): boolean {
  if (storedFollowSystem !== null) return storedFollowSystem === "true";
  return storedTheme === null;
}

/** Return the complete semantic variable map for a first-party theme. */
export function createSchoolXTheme(name: SchoolXThemeName): ThemeResult {
  const isDark = name === "buzz-dark";
  return {
    isDark,
    vars: { ...(isDark ? SCHOOLX_DARK_VARS : SCHOOLX_LIGHT_VARS) },
  };
}

/** Parse and structurally validate a synchronous theme cache payload. */
export function parseThemeCachePayload(raw: string): ThemeCachePayload | null {
  try {
    const value: unknown = JSON.parse(raw);
    if (!value || typeof value !== "object") return null;
    const candidate = value as Partial<ThemeCachePayload>;
    if (typeof candidate.themeName !== "string") return null;
    if (typeof candidate.isDark !== "boolean") return null;
    if (
      !candidate.vars ||
      typeof candidate.vars !== "object" ||
      Array.isArray(candidate.vars)
    ) {
      return null;
    }
    if (
      !Object.entries(candidate.vars).every(
        ([key, variable]) =>
          key.startsWith("--") && typeof variable === "string",
      )
    ) {
      return null;
    }
    if (
      candidate.revision !== undefined &&
      typeof candidate.revision !== "string"
    ) {
      return null;
    }
    return candidate as ThemeCachePayload;
  } catch {
    return null;
  }
}

/**
 * Decide whether prepaint may use a cache for the current effective theme.
 * Third-party caches retain their historical revisionless behavior.
 */
export function canApplyThemeCache(
  cached: ThemeCachePayload,
  expectedThemeName: string,
): boolean {
  if (isSchoolXThemeName(expectedThemeName)) {
    const expected = createSchoolXTheme(expectedThemeName);
    return (
      cached.themeName === expectedThemeName &&
      cached.revision === SCHOOLX_THEME_CACHE_REVISION &&
      cached.isDark === expected.isDark &&
      Object.keys(cached.vars).length === Object.keys(expected.vars).length &&
      Object.entries(expected.vars).every(
        ([key, value]) => cached.vars[key] === value,
      )
    );
  }
  return !isSchoolXThemeName(cached.themeName);
}
