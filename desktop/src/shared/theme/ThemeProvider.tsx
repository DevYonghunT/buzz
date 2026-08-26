import {
  type ReactNode,
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from "react";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invokeTauri } from "@/shared/api/tauri";
import { createThemeVars, hexToHsl } from "./adaptive-theme";
import {
  SYNTAX_THEMES,
  type SyntaxThemeName,
  extractThemeInfo,
  getThemePair,
  loadThemeData,
  resolveSystemTheme,
} from "./theme-loader";
import {
  SCHOOLX_PALETTE,
  SCHOOLX_THEME_CACHE_REVISION,
  SCHOOLX_THEME_ONLY_VAR_NAMES,
  canApplyThemeCache,
  createSchoolXTheme,
  isSchoolXThemeName,
  parseThemeCachePayload,
  resolveStoredFollowSystem,
  resolveStoredThemeName,
} from "./schoolx-theme";

export const THEME_STORAGE_KEY = "buzz-theme";
const CACHE_KEY = "buzz-theme-cache";
export const ACCENT_STORAGE_KEY = "buzz-accent-color";
export const NEUTRAL_ACCENT = "neutral";
const FOLLOW_SYSTEM_KEY = "buzz-follow-system";
const VIDEO_REVIEW_NEUTRAL_ACCENT = "0 0% 98%";
const VIDEO_REVIEW_CHIP_SURFACE = "#161616";
const VIDEO_REVIEW_TEXT_CONTRAST = 4.5;
const VIDEO_REVIEW_CHIP_BACKGROUND_ALPHAS = [0.15, 0.3] as const;
const BUZZ_VIBRANCY_MATERIAL = "sidebar";

export const ACCENT_COLORS = [
  { name: "Neutral", value: NEUTRAL_ACCENT },
  { name: "Blue", value: "#3b82f6" },
  { name: "Cyan", value: "#06b6d4" },
  { name: "Green", value: "#22c55e" },
  { name: "Orange", value: "#f97316" },
  { name: "Red", value: "#ef4444" },
  { name: "Pink", value: "#ec4899" },
  { name: "Lilac", value: "#c0a2f1" },
  { name: "Purple", value: "#a855f7" },
  { name: "Indigo", value: "#6366f1" },
] as const;

const DEFAULT_ACCENT = "#3b82f6";

type ThemeContextValue = {
  themeName: string;
  selectedThemeName: string;
  isDark: boolean;
  isLoading: boolean;
  accentColor: string;
  followSystem: boolean;
  hasPair: boolean;
  setTheme: (name: string) => void;
  setAccentColor: (color: string) => void;
  setFollowSystem: (enabled: boolean) => void;
};

type ThemeProviderProps = {
  children: ReactNode;
  defaultTheme?: SyntaxThemeName;
};

const ThemeContext = createContext<ThemeContextValue | undefined>(undefined);

function isValidThemeName(name: string): name is SyntaxThemeName {
  return (SYNTAX_THEMES as readonly string[]).includes(name);
}

/** Read stored theme, migrating legacy "light"/"dark"/"system" values. */
function readStoredTheme(fallback: SyntaxThemeName): SyntaxThemeName {
  const stored = window.localStorage.getItem(THEME_STORAGE_KEY);
  return resolveStoredThemeName(
    stored,
    fallback,
    isValidThemeName,
  ) as SyntaxThemeName;
}

function getContrastColor(hex: string): string {
  const m = /^#?([a-f\d]{2})([a-f\d]{2})([a-f\d]{2})/i.exec(hex);
  if (!m) return "#ffffff";
  const r = parseInt(m[1], 16);
  const g = parseInt(m[2], 16);
  const b = parseInt(m[3], 16);
  const lum = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
  return lum > 0.5 ? "#000000" : "#ffffff";
}

type Rgb = {
  r: number;
  g: number;
  b: number;
};

function hexToRgb(hex: string): Rgb {
  const m = /^#?([a-f\d]{2})([a-f\d]{2})([a-f\d]{2})/i.exec(hex);
  if (!m) return { r: 255, g: 255, b: 255 };
  return {
    r: parseInt(m[1], 16),
    g: parseInt(m[2], 16),
    b: parseInt(m[3], 16),
  };
}

function mixRgb(from: Rgb, to: Rgb, factor: number): Rgb {
  return {
    r: from.r + (to.r - from.r) * factor,
    g: from.g + (to.g - from.g) * factor,
    b: from.b + (to.b - from.b) * factor,
  };
}

function compositeRgb(foreground: Rgb, background: Rgb, alpha: number): Rgb {
  return mixRgb(background, foreground, alpha);
}

function relativeLuminance({ r, g, b }: Rgb): number {
  const [rs, gs, bs] = [r, g, b].map((channel) => {
    const value = channel / 255;
    return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * rs + 0.7152 * gs + 0.0722 * bs;
}

function contrastRatio(a: Rgb, b: Rgb): number {
  const aLum = relativeLuminance(a);
  const bLum = relativeLuminance(b);
  return (Math.max(aLum, bLum) + 0.05) / (Math.min(aLum, bLum) + 0.05);
}

function getReviewAccentForeground(hex: string): string {
  const accent = hexToRgb(hex);
  const surface = hexToRgb(VIDEO_REVIEW_CHIP_SURFACE);
  const white = { r: 255, g: 255, b: 255 };
  const backgrounds = VIDEO_REVIEW_CHIP_BACKGROUND_ALPHAS.map((alpha) =>
    compositeRgb(accent, surface, alpha),
  );
  let low = 0;
  let high = 1;

  for (let i = 0; i < 20; i++) {
    const mid = (low + high) / 2;
    const candidate = mixRgb(accent, white, mid);
    const minContrast = Math.min(
      ...backgrounds.map((background) => contrastRatio(candidate, background)),
    );

    if (minContrast >= VIDEO_REVIEW_TEXT_CONTRAST) {
      high = mid;
    } else {
      low = mid;
    }
  }

  return hexToHsl(rgbToHex(mixRgb(accent, white, high)));
}

function rgbToHex({ r, g, b }: Rgb): string {
  const clamp = (value: number) =>
    Math.max(0, Math.min(255, Math.round(value)));
  return `#${[r, g, b]
    .map((channel) => clamp(channel).toString(16).padStart(2, "0"))
    .join("")}`;
}

function applyThirdPartyAccentColor(value: string) {
  const root = document.documentElement;
  if (value === NEUTRAL_ACCENT) {
    const styles = window.getComputedStyle(root);
    const foreground = styles.getPropertyValue("--foreground").trim();
    const background = styles.getPropertyValue("--background").trim();
    root.style.setProperty("--buzz-selected-accent", foreground);
    root.style.setProperty(
      "--buzz-video-review-accent",
      VIDEO_REVIEW_NEUTRAL_ACCENT,
    );
    root.style.setProperty(
      "--buzz-video-review-accent-foreground",
      VIDEO_REVIEW_NEUTRAL_ACCENT,
    );
    root.style.setProperty("--primary", foreground);
    root.style.setProperty("--primary-foreground", background);
    root.style.setProperty("--sidebar-primary", foreground);
    root.style.setProperty("--sidebar-primary-foreground", background);
    root.style.setProperty("--sidebar-active", foreground);
    root.style.setProperty("--sidebar-active-foreground", background);
    return;
  }

  const hex = value;
  const accentHsl = hexToHsl(hex);
  const fgHsl = hexToHsl(getContrastColor(hex));
  root.style.setProperty("--buzz-selected-accent", accentHsl);
  root.style.setProperty("--buzz-video-review-accent", accentHsl);
  root.style.setProperty(
    "--buzz-video-review-accent-foreground",
    getReviewAccentForeground(hex),
  );
  root.style.setProperty("--primary", accentHsl);
  root.style.setProperty("--primary-foreground", fgHsl);
  root.style.setProperty("--sidebar-primary", accentHsl);
  root.style.setProperty("--sidebar-primary-foreground", fgHsl);
  root.style.setProperty("--sidebar-active", accentHsl);
  root.style.setProperty("--sidebar-active-foreground", fgHsl);
}

/**
 * The internal Buzz compatibility IDs select the fixed SchoolX palette. The
 * stored third-party accent stays untouched so it returns with that theme.
 */
export function isBuzzTheme(themeName: string): boolean {
  return isSchoolXThemeName(themeName);
}

function applyEffectiveAccent(themeName: string, accentColor: string) {
  if (!isSchoolXThemeName(themeName)) {
    applyThirdPartyAccentColor(accentColor);
    return;
  }

  const root = document.documentElement;
  const { vars } = createSchoolXTheme(themeName);
  const primary = vars["--primary"];
  const primaryForeground = vars["--primary-foreground"];
  root.style.setProperty("--buzz-selected-accent", primary);
  root.style.setProperty("--buzz-video-review-accent", primary);
  root.style.setProperty(
    "--buzz-video-review-accent-foreground",
    getReviewAccentForeground(
      themeName === "buzz-dark"
        ? SCHOOLX_PALETTE.terracottaDark
        : SCHOOLX_PALETTE.pine,
    ),
  );
  root.style.setProperty("--primary", primary);
  root.style.setProperty("--primary-foreground", primaryForeground);
  root.style.setProperty("--sidebar-primary", vars["--sidebar-primary"]);
  root.style.setProperty(
    "--sidebar-primary-foreground",
    vars["--sidebar-primary-foreground"],
  );
  root.style.setProperty("--sidebar-active", vars["--sidebar-active"]);
  root.style.setProperty(
    "--sidebar-active-foreground",
    vars["--sidebar-active-foreground"],
  );
}

/** Preserve the existing first-party DOM markers while keeping V1 opaque. */
function applyBuzzSidebar(themeName: string) {
  const root = document.documentElement;
  root.removeAttribute("data-buzz-translucent");
  if (isBuzzTheme(themeName)) {
    root.setAttribute("data-buzz-sidebar", "");
    root.setAttribute("data-buzz-theme", themeName);
  } else {
    root.removeAttribute("data-buzz-sidebar");
    root.removeAttribute("data-buzz-theme");
  }
}

/** Best-effort removal of the legacy layer; CSS remains opaque on rejection. */
async function applyBuzzVibrancy(_themeName: string) {
  document.documentElement.removeAttribute("data-buzz-translucent");
  if (!isTauri()) return;
  try {
    await invokeTauri<void>("set_window_vibrancy", {
      enabled: false,
      material: BUZZ_VIBRANCY_MATERIAL,
    });
  } catch (error) {
    console.warn("set_window_vibrancy failed", error);
  }
}

function clearSchoolXOnlyVars() {
  const root = document.documentElement;
  for (const variable of SCHOOLX_THEME_ONLY_VAR_NAMES) {
    root.style.removeProperty(variable);
  }
}

function applyResolvedTheme(
  themeName: SyntaxThemeName,
  isDark: boolean,
  vars: Record<string, string>,
) {
  const root = document.documentElement;
  clearSchoolXOnlyVars();
  for (const [key, value] of Object.entries(vars)) {
    root.style.setProperty(key, value);
  }
  root.classList.remove("light", "dark");
  root.classList.add(isDark ? "dark" : "light");
  const background = vars["--background"];
  if (background) root.style.backgroundColor = `hsl(${background})`;
  else root.style.removeProperty("background-color");
  applyBuzzSidebar(themeName);
  applyEffectiveAccent(
    themeName,
    window.localStorage.getItem(ACCENT_STORAGE_KEY) ?? DEFAULT_ACCENT,
  );
}

/** Apply a compatible cache synchronously to prevent FOUC. */
function applyCachedVars(expectedThemeName: SyntaxThemeName): string | null {
  try {
    const cached = window.localStorage.getItem(CACHE_KEY);
    if (!cached) return null;
    const payload = parseThemeCachePayload(cached);
    if (!payload || !isValidThemeName(payload.themeName)) return null;
    if (!canApplyThemeCache(payload, expectedThemeName)) return null;
    applyResolvedTheme(payload.themeName, payload.isDark, payload.vars);
    return payload.themeName;
  } catch {
    return null;
  }
}

/** The latest theme load is the only one allowed to write document styles. */
let themeApplyRequest = 0;

/** Apply a theme: load data, derive CSS vars, set them on :root. */
async function applyTheme(
  name: SyntaxThemeName,
): Promise<{ isDark: boolean } | null> {
  const requestToken = ++themeApplyRequest;
  let result: { isDark: boolean; vars: Record<string, string> };
  if (isSchoolXThemeName(name)) {
    result = createSchoolXTheme(name);
  } else {
    const themeData = await loadThemeData(name);
    if (requestToken !== themeApplyRequest) return null;
    const info = extractThemeInfo(name, themeData);
    result = createThemeVars(info.bg, info.fg, info.comment, {
      added: info.added,
      deleted: info.deleted,
      modified: info.modified,
    });
  }
  if (requestToken !== themeApplyRequest) return null;

  applyResolvedTheme(name, result.isDark, result.vars);

  try {
    const payload = isSchoolXThemeName(name)
      ? {
          isDark: result.isDark,
          revision: SCHOOLX_THEME_CACHE_REVISION,
          themeName: name,
          vars: result.vars,
        }
      : { isDark: result.isDark, themeName: name, vars: result.vars };
    window.localStorage.setItem(CACHE_KEY, JSON.stringify(payload));
  } catch {
    // Storage full — non-critical
  }

  return { isDark: result.isDark };
}

type InitialThemeState = {
  followSystem: boolean;
  selectedTheme: SyntaxThemeName;
  systemIsDark: boolean;
};

function initializeThemeState(
  defaultTheme: SyntaxThemeName,
): InitialThemeState {
  const storedTheme = window.localStorage.getItem(THEME_STORAGE_KEY);
  const selectedTheme = readStoredTheme(defaultTheme);
  const followSystem = resolveStoredFollowSystem(
    window.localStorage.getItem(FOLLOW_SYSTEM_KEY),
    storedTheme,
  );
  const systemIsDark = window.matchMedia(
    "(prefers-color-scheme: dark)",
  ).matches;
  const effectiveTheme = followSystem
    ? resolveSystemTheme(selectedTheme, systemIsDark)
    : selectedTheme;

  if (!applyCachedVars(effectiveTheme)) {
    if (isSchoolXThemeName(effectiveTheme)) {
      const { isDark, vars } = createSchoolXTheme(effectiveTheme);
      applyResolvedTheme(effectiveTheme, isDark, vars);
    } else {
      clearSchoolXOnlyVars();
      applyBuzzSidebar(effectiveTheme);
    }
  }

  return { followSystem, selectedTheme, systemIsDark };
}

export function ThemeProvider({
  children,
  defaultTheme = "buzz",
}: ThemeProviderProps) {
  const [initialThemeState] = useState(() =>
    initializeThemeState(defaultTheme),
  );
  const [selectedTheme, setSelectedTheme] = useState<string>(
    initialThemeState.selectedTheme,
  );
  const [isDark, setIsDark] = useState<boolean>(() => {
    return document.documentElement.classList.contains("dark");
  });
  const [isLoading, setIsLoading] = useState(true);
  const loadingRef = useRef<string | null>(null);
  const [accentColor, setAccentColorState] = useState<string>(() => {
    return window.localStorage.getItem(ACCENT_STORAGE_KEY) ?? DEFAULT_ACCENT;
  });
  const [followSystem, setFollowSystemState] = useState<boolean>(
    initialThemeState.followSystem,
  );
  const [systemIsDark, setSystemIsDark] = useState<boolean>(
    initialThemeState.systemIsDark,
  );

  // Resolve the effective theme based on follow-system preference
  const effectiveTheme = (() => {
    if (!followSystem || !isValidThemeName(selectedTheme)) return selectedTheme;
    return resolveSystemTheme(selectedTheme as SyntaxThemeName, systemIsDark);
  })();

  // Check if the selected theme has a pair (for UI hint)
  const hasPair = isValidThemeName(selectedTheme)
    ? getThemePair(selectedTheme as SyntaxThemeName) !== null
    : false;

  useEffect(() => {
    if (!isValidThemeName(effectiveTheme)) return;

    // Track which theme we're loading to avoid race conditions
    const thisTheme = effectiveTheme;
    loadingRef.current = thisTheme;
    setIsLoading(true);

    applyTheme(effectiveTheme as SyntaxThemeName).then((result) => {
      if (!result) return;
      // Only update if this is still the theme we want. The accent is applied
      // inside applyTheme (synchronously with the theme vars), so there's no
      // separate re-application here — that avoided the switch-time flicker.
      if (loadingRef.current === thisTheme) {
        setIsDark(result.isDark);
        setIsLoading(false);
      }
    });
  }, [effectiveTheme]);

  useEffect(() => {
    if (!isValidThemeName(effectiveTheme)) return;
    void applyBuzzVibrancy(effectiveTheme);
  }, [effectiveTheme]);

  // Listen for system color scheme changes when followSystem is enabled
  useEffect(() => {
    if (!followSystem) return;

    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const handleMediaChange = (event: MediaQueryListEvent) => {
      setSystemIsDark(event.matches);
    };
    let disposed = false;
    let unlistenNativeTheme: (() => void) | undefined;

    setSystemIsDark(mq.matches);
    mq.addEventListener("change", handleMediaChange);

    // WKWebView can update the media query value without dispatching its
    // change event until the page reloads. Tauri's native window event arrives
    // immediately when macOS appearance changes, so use it as the reliable app
    // signal while retaining matchMedia for the browser build.
    if (isTauri()) {
      void getCurrentWindow()
        .onThemeChanged(({ payload }) => {
          if (!disposed) setSystemIsDark(payload === "dark");
        })
        .then((unlisten) => {
          if (disposed) {
            unlisten();
          } else {
            unlistenNativeTheme = unlisten;
          }
        })
        .catch((error) => {
          console.warn("system theme listener unavailable", error);
        });
    }

    return () => {
      disposed = true;
      mq.removeEventListener("change", handleMediaChange);
      unlistenNativeTheme?.();
    };
  }, [followSystem]);

  // First-party themes reassert their fixed semantic action colors here;
  // third-party themes continue to use the stored accent swatch.
  useEffect(() => {
    applyEffectiveAccent(effectiveTheme, accentColor);
  }, [accentColor, effectiveTheme]);

  const setTheme = useCallback((name: string) => {
    if (!isValidThemeName(name)) return;
    setSelectedTheme(name);
    window.localStorage.setItem(THEME_STORAGE_KEY, name);
  }, []);

  const setAccentColor = useCallback((color: string) => {
    window.localStorage.setItem(ACCENT_STORAGE_KEY, color);
    setAccentColorState(color);
  }, []);

  const setFollowSystem = useCallback((enabled: boolean) => {
    window.localStorage.setItem(FOLLOW_SYSTEM_KEY, enabled ? "true" : "false");
    setFollowSystemState(enabled);
  }, []);

  const value: ThemeContextValue = {
    themeName: effectiveTheme,
    selectedThemeName: selectedTheme,
    isDark,
    isLoading,
    accentColor,
    followSystem,
    hasPair,
    setTheme,
    setAccentColor,
    setFollowSystem,
  };

  return (
    <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>
  );
}

export function useTheme() {
  const context = useContext(ThemeContext);
  if (!context) {
    throw new Error("useTheme must be used within a ThemeProvider");
  }
  return context;
}
