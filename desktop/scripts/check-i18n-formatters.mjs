import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { globSync } from "node:fs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, "..");

// Date, time and number text must follow the *app's* language setting, which
// lives in `shared/i18n`, not a literal baked into a module and not the OS.
//
// Two failure modes this catches, both of which shipped:
//
//   new Intl.DateTimeFormat("en-US", …)   — frozen English. A Korean user sees
//                                            English dates no matter what they
//                                            pick in settings.
//   date.toLocaleString(undefined, …)     — follows the OS. A user on an English
//   date.toLocaleDateString()                Mac who chose 한국어 still gets
//                                            English dates, and two screens can
//                                            disagree on the same page.
//
// There is a third, quieter one the guard cannot see but the fix removes:
// a module-level `const F = new Intl.DateTimeFormat(...)` resolves its locale
// once at import and can never react to a language change, so even passing the
// right locale at that line would only be correct until the user switches.
// Build formatters per render from the current locale instead —
// `shared/i18n/formatters` caches them by locale and options.
const BANNED = [
  {
    id: "intl-literal-locale",
    // new Intl.DateTimeFormat("en-US" | 'ko-KR' | `…`
    pattern:
      /new\s+Intl\.(?:DateTimeFormat|NumberFormat|RelativeTimeFormat|ListFormat|PluralRules|Collator)\s*\(\s*["'`]/g,
    message:
      "hardcoded locale — use shared/i18n/formatters with the locale from useAppLocale()",
  },
  {
    id: "intl-implicit-locale",
    pattern:
      /new\s+Intl\.(?:DateTimeFormat|NumberFormat|RelativeTimeFormat|ListFormat|PluralRules|Collator)\s*\(\s*(?:undefined|\))/g,
    message:
      "implicit OS locale — pass the app locale via shared/i18n/formatters",
  },
  {
    id: "to-locale-string",
    pattern: /\.toLocale(?:Date|Time)?String\s*\(/g,
    message:
      "toLocale*String follows the OS locale — use shared/i18n/formatters instead",
  },
];

// `shared/i18n` is where the app locale is resolved and where the only
// legitimate `new Intl.*` construction lives.
const EXEMPT_DIRS = ["src/shared/i18n/"];

function isExempt(relativePath) {
  if (EXEMPT_DIRS.some((dir) => relativePath.startsWith(dir))) {
    return true;
  }
  return /\.test\.mjs$|\.test\.ts$|\.spec\.ts$/.test(relativePath);
}

// Call sites not yet converted, as `relativePath:ruleId`. Every entry is a
// screen that still renders dates in the wrong language; they are recorded
// rather than fixed so the conversion is a bounded, checkable list instead of an
// open-ended sweep. Removing an entry must make this check pass, never fail —
// delete the line when you convert the file. Do not add new entries.
const PENDING_CONVERSION = new Set([
  "src/features/agents/ui/AgentSessionTranscriptList.tsx:to-locale-string",
  "src/features/agents/ui/agentSessionUtils.ts:intl-implicit-locale",
  "src/features/agents/ui/agentSessionUtils.ts:intl-literal-locale",
  "src/features/channels/lib/ephemeralChannel.ts:intl-literal-locale",
  "src/features/channels/ui/AgentSessionThreadPanel.tsx:to-locale-string",
  "src/features/community-members/ui/CommunityMembersCard.tsx:to-locale-string",
  "src/features/community-members/ui/CommunityMembersSettingsCard.tsx:to-locale-string",
  "src/features/home/lib/inbox.ts:intl-literal-locale",
  "src/features/home/ui/FeedSection.tsx:intl-literal-locale",
  "src/features/messages/lib/dateFormatters.ts:intl-literal-locale",
  "src/features/messages/ui/DraftsPanel.tsx:intl-literal-locale",
  "src/features/projects/lib/projectsViewHelpers.ts:to-locale-string",
  "src/features/projects/ui/ProjectCommitDetailPanel.tsx:to-locale-string",
  "src/features/projects/ui/ProjectDetailFeedPanels.tsx:to-locale-string",
  "src/features/projects/ui/ProjectRepositoryPanel.tsx:to-locale-string",
  "src/features/projects/ui/ProjectsContributionGraph.tsx:to-locale-string",
  "src/features/projects/ui/ProjectsIssuesList.tsx:to-locale-string",
  "src/features/projects/ui/ProjectsPullRequestsList.tsx:to-locale-string",
  "src/features/pulse/ui/AgentActivityCard.tsx:to-locale-string",
  "src/features/pulse/ui/NoteCard.tsx:to-locale-string",
  "src/features/search/ui/TopbarSearch.tsx:intl-literal-locale",
  "src/features/settings/ui/ModerationQueueCard.tsx:to-locale-string",
  "src/features/workflows/ui/WorkflowApprovalCard.tsx:to-locale-string",
  "src/features/workflows/ui/WorkflowCard.tsx:to-locale-string",
  "src/features/workflows/ui/WorkflowDetailPanel.tsx:to-locale-string",
]);

const files = globSync("src/**/*.{ts,tsx}", { cwd: projectRoot }).sort();
const violations = [];
const seen = new Set();

for (const relativePath of files) {
  if (isExempt(relativePath)) {
    continue;
  }

  const source = readFileSync(path.join(projectRoot, relativePath), "utf8");
  const lines = source.split("\n");

  for (const rule of BANNED) {
    for (const [index, line] of lines.entries()) {
      // Comments describing the rule are not violations of it.
      if (/^\s*(?:\/\/|\*|\/\*)/.test(line)) {
        continue;
      }
      rule.pattern.lastIndex = 0;
      if (!rule.pattern.test(line)) {
        continue;
      }

      const key = `${relativePath}:${rule.id}`;
      seen.add(key);
      if (PENDING_CONVERSION.has(key)) {
        continue;
      }
      violations.push({
        line: index + 1,
        message: rule.message,
        relativePath,
      });
    }
  }
}

// A stale allowlist is its own bug: it hides the next regression in a file
// somebody already fixed.
const staleEntries = [...PENDING_CONVERSION].filter((key) => !seen.has(key));

if (violations.length === 0 && staleEntries.length === 0) {
  console.log(
    `Desktop i18n formatter check passed (${files.length} files, ${PENDING_CONVERSION.size} awaiting conversion).`,
  );
  process.exit(0);
}

if (violations.length > 0) {
  console.error("Desktop i18n formatter check failed.\n");
  for (const violation of violations) {
    console.error(
      `  ${violation.relativePath}:${violation.line} — ${violation.message}`,
    );
  }
  console.error(
    "\nFormat through shared/i18n/formatters with the locale from useAppLocale(),",
  );
  console.error(
    "so the text follows the language the user picked in settings.",
  );
}

if (staleEntries.length > 0) {
  console.error(
    "\nStale entries in PENDING_CONVERSION (file no longer violates — delete the line):\n",
  );
  for (const entry of staleEntries) {
    console.error(`  ${entry}`);
  }
  console.error(
    `\nEdit the list in desktop/scripts/check-i18n-formatters.mjs.`,
  );
}

process.exit(1);
