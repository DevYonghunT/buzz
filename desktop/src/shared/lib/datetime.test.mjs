import assert from "node:assert/strict";
import test from "node:test";

import { formatDayGroupLabel, formatItemTimestamp } from "./datetime.ts";

/** Local-time unix seconds, so the tests read in the same zone the code uses. */
function at(year, monthIndex, day, hour = 12, minute = 0) {
  return new Date(year, monthIndex, day, hour, minute).getTime() / 1_000;
}

const NOW = at(2026, 6, 30, 14, 30); // Thu Jul 30 2026, 2:30 PM local

test("the same calendar day reads Today", () => {
  assert.equal(formatDayGroupLabel(at(2026, 6, 30, 9, 5), NOW), "Today");
  // Just after midnight and just before it still count as the same day.
  assert.equal(formatDayGroupLabel(at(2026, 6, 30, 0, 1), NOW), "Today");
  assert.equal(formatDayGroupLabel(at(2026, 6, 30, 23, 59), NOW), "Today");
});

test("the previous calendar day reads Yesterday", () => {
  assert.equal(formatDayGroupLabel(at(2026, 6, 29, 23, 59), NOW), "Yesterday");
  assert.equal(formatDayGroupLabel(at(2026, 6, 29, 0, 0), NOW), "Yesterday");
});

test("Yesterday is a calendar boundary, not a 24-hour window", () => {
  // 15 hours earlier, but the day rolled over: yesterday, not Today.
  const lateLastNight = at(2026, 6, 29, 23, 30);
  assert.equal(formatDayGroupLabel(lateLastNight, NOW), "Yesterday");
  // 22 hours earlier and still the same calendar day: Today.
  const earlyToday = at(2026, 6, 30, 0, 30);
  assert.equal(formatDayGroupLabel(earlyToday, NOW), "Today");
});

test("two to six days back reads as the weekday alone", () => {
  assert.equal(formatDayGroupLabel(at(2026, 6, 28), NOW), "Tuesday");
  assert.equal(formatDayGroupLabel(at(2026, 6, 27), NOW), "Monday");
  assert.equal(formatDayGroupLabel(at(2026, 6, 24), NOW), "Friday");
});

test("seven days back leaves the weekday band, so it can't repeat a name", () => {
  // Same weekday name as today — a weekday label here would be ambiguous.
  assert.equal(formatDayGroupLabel(at(2026, 6, 23), NOW), "Thursday, July 23");
});

test("older dates in the current year omit the year", () => {
  assert.equal(formatDayGroupLabel(at(2026, 5, 20), NOW), "Saturday, June 20");
  assert.equal(formatDayGroupLabel(at(2026, 0, 3), NOW), "Saturday, January 3");
});

test("dates in earlier years include the year", () => {
  assert.equal(formatDayGroupLabel(at(2025, 5, 20), NOW), "June 20, 2025");
  assert.equal(formatDayGroupLabel(at(2022, 7, 22), NOW), "August 22, 2022");
});

test("old dates keep the day, so consecutive dividers stay distinguishable", () => {
  // The standard would collapse these to "Aug 2022"; a group label must
  // identify its own day.
  const labels = [
    formatDayGroupLabel(at(2022, 7, 21), NOW),
    formatDayGroupLabel(at(2022, 7, 22), NOW),
    formatDayGroupLabel(at(2022, 7, 23), NOW),
  ];
  assert.equal(new Set(labels).size, 3, `labels repeated: ${labels}`);
});

test("no label carries an ordinal suffix", () => {
  const probes = [
    at(2026, 6, 30),
    at(2026, 6, 29),
    at(2026, 6, 27),
    at(2026, 5, 1),
    at(2026, 5, 2),
    at(2026, 5, 3),
    at(2026, 5, 11),
    at(2026, 5, 12),
    at(2026, 5, 13),
    at(2026, 5, 21),
    at(2026, 5, 22),
    at(2026, 5, 23),
    at(2025, 5, 20),
  ];
  for (const probe of probes) {
    assert.doesNotMatch(
      formatDayGroupLabel(probe, NOW),
      /\d(?:st|nd|rd|th)\b/,
      `ordinal suffix in ${formatDayGroupLabel(probe, NOW)}`,
    );
  }
});

test("a future timestamp is not labelled with a past weekday", () => {
  // Clock skew, or a relay running ahead. "Tuesday" would read as last Tuesday.
  const nextWeek = at(2026, 7, 4);
  assert.equal(formatDayGroupLabel(nextWeek, NOW), "Tuesday, August 4");
  // Still same-day, so Today remains correct for small skew.
  assert.equal(formatDayGroupLabel(at(2026, 6, 30, 23, 0), NOW), "Today");
});

test("the label follows the current clock, not a captured one", () => {
  const event = at(2026, 6, 30, 9, 0);
  assert.equal(formatDayGroupLabel(event, NOW), "Today");
  // Same event, read a day later.
  assert.equal(formatDayGroupLabel(event, at(2026, 6, 31, 9, 0)), "Yesterday");
});

test("an explicit English locale preserves the upstream group-label ladder", () => {
  const options = { locale: "en", nowSeconds: NOW };
  assert.equal(formatDayGroupLabel(at(2026, 6, 30), options), "Today");
  assert.equal(formatDayGroupLabel(at(2026, 6, 29), options), "Yesterday");
  assert.equal(formatDayGroupLabel(at(2026, 6, 27), options), "Monday");
  assert.equal(
    formatDayGroupLabel(at(2026, 5, 20), options),
    "Saturday, June 20",
  );
  assert.equal(formatDayGroupLabel(at(2025, 5, 20), options), "June 20, 2025");
  assert.equal(
    formatItemTimestamp(at(2026, 6, 29, 9, 5), {
      ...options,
      withTime: true,
    }),
    "Yesterday at 9:05 AM",
  );
});

test("Korean group labels keep the same relative-date bands", () => {
  const options = { locale: "ko", nowSeconds: NOW };
  assert.equal(formatDayGroupLabel(at(2026, 6, 30), options), "오늘");
  assert.equal(formatDayGroupLabel(at(2026, 6, 29), options), "어제");
  assert.equal(formatDayGroupLabel(at(2026, 6, 27), options), "월요일");
  assert.equal(
    formatDayGroupLabel(at(2026, 6, 23), options),
    "7월 23일 목요일",
  );
  assert.equal(
    formatDayGroupLabel(at(2026, 5, 20), options),
    "6월 20일 토요일",
  );
  assert.equal(
    formatDayGroupLabel(at(2025, 5, 20), options),
    "2025년 6월 20일",
  );
});

// ── formatItemTimestamp ─────────────────────────────────────────────────────

test("today is a bare clock time in both modes", () => {
  const today = at(2026, 6, 30, 9, 5);
  assert.equal(formatItemTimestamp(today, { nowSeconds: NOW }), "9:05 AM");
  assert.equal(
    formatItemTimestamp(today, { withTime: true, nowSeconds: NOW }),
    "9:05 AM",
  );
});

test("compact mode drops the time outside today", () => {
  const opts = { nowSeconds: NOW };
  assert.equal(formatItemTimestamp(at(2026, 6, 29, 9, 5), opts), "Yesterday");
  assert.equal(formatItemTimestamp(at(2026, 6, 27, 9, 5), opts), "Monday");
  assert.equal(formatItemTimestamp(at(2026, 5, 20, 9, 5), opts), "Sat, Jun 20");
  assert.equal(
    formatItemTimestamp(at(2025, 5, 20, 9, 5), opts),
    "Jun 20, 2025",
  );
});

test("roomy mode keeps the time at every band, joined with 'at'", () => {
  const opts = { withTime: true, nowSeconds: NOW };
  assert.equal(
    formatItemTimestamp(at(2026, 6, 29, 9, 5), opts),
    "Yesterday at 9:05 AM",
  );
  assert.equal(
    formatItemTimestamp(at(2026, 6, 27, 14, 34), opts),
    "Monday at 2:34 PM",
  );
  assert.equal(
    formatItemTimestamp(at(2026, 5, 20, 14, 34), opts),
    "Sat, Jun 20 at 2:34 PM",
  );
  assert.equal(
    formatItemTimestamp(at(2025, 5, 20, 14, 34), opts),
    "Jun 20, 2025 at 2:34 PM",
  );
});

test("Korean item labels localize every band without the English 'at' joiner", () => {
  const compact = { locale: "ko", nowSeconds: NOW };
  assert.equal(formatItemTimestamp(at(2026, 6, 29, 9, 5), compact), "어제");
  assert.equal(formatItemTimestamp(at(2026, 6, 27, 14, 34), compact), "월요일");
  assert.equal(
    formatItemTimestamp(at(2026, 5, 20, 14, 34), compact),
    "6월 20일 (토)",
  );
  assert.equal(
    formatItemTimestamp(at(2025, 5, 20, 14, 34), compact),
    "2025년 6월 20일",
  );

  const roomy = { locale: "ko", withTime: true, nowSeconds: NOW };
  for (const [timestamp, prefix, clock] of [
    [at(2026, 6, 29, 9, 5), "어제 ", "9:05"],
    [at(2026, 6, 27, 14, 34), "월요일 ", "2:34"],
    [at(2026, 5, 20, 14, 34), "6월 20일 (토) ", "2:34"],
    [at(2025, 5, 20, 14, 34), "2025년 6월 20일 ", "2:34"],
  ]) {
    const label = formatItemTimestamp(timestamp, roomy);
    assert.ok(label.startsWith(prefix), `missing Korean prefix in "${label}"`);
    assert.ok(label.endsWith(clock), `missing clock time in "${label}"`);
    assert.doesNotMatch(
      label,
      /\bat\b/,
      `English joiner leaked into "${label}"`,
    );
  }
});

test("Korean date labels never inherit an English ordinal suffix", () => {
  for (const formatter of [
    (timestamp) =>
      formatDayGroupLabel(timestamp, { locale: "ko", nowSeconds: NOW }),
    (timestamp) =>
      formatItemTimestamp(timestamp, { locale: "ko", nowSeconds: NOW }),
  ]) {
    for (const day of [1, 2, 3, 11, 12, 13, 21, 22, 23, 31]) {
      assert.doesNotMatch(formatter(at(2025, 0, day)), /\d(?:st|nd|rd|th)\b/);
    }
  }
});

test("the year is omitted within the current year in both modes", () => {
  for (const withTime of [false, true]) {
    const label = formatItemTimestamp(at(2026, 0, 3, 9, 5), {
      withTime,
      nowSeconds: NOW,
    });
    assert.doesNotMatch(label, /2026/, `current year leaked into "${label}"`);
  }
});

test("no item label carries an ordinal suffix", () => {
  for (const withTime of [false, true]) {
    for (const day of [1, 2, 3, 11, 12, 13, 21, 22, 23, 31]) {
      const label = formatItemTimestamp(at(2026, 0, day, 9, 5), {
        withTime,
        nowSeconds: NOW,
      });
      assert.doesNotMatch(
        label,
        /\d(?:st|nd|rd|th)\b/,
        `ordinal in "${label}"`,
      );
    }
  }
});

test("compact labels stay short enough for a narrow list row", () => {
  for (const probe of [
    at(2026, 6, 30, 14, 34),
    at(2026, 6, 29),
    at(2026, 6, 27),
    at(2026, 5, 20),
    at(2025, 5, 20),
  ]) {
    const label = formatItemTimestamp(probe, { nowSeconds: NOW });
    assert.ok(label.length <= 12, `"${label}" is ${label.length} chars`);
  }
});
