import assert from "node:assert/strict";
import test from "node:test";

import {
  formatDayHeading,
  formatShortMonthDayOrdinal,
  formatThreadSummaryLastReplyTime,
  formatTime,
  formatTimeWithoutDayPeriod,
  startOfLocalDaySeconds,
} from "./dateFormatters.ts";

function localUnixSeconds(year, monthIndex, day, hour = 12, minute = 0) {
  return new Date(year, monthIndex, day, hour, minute).getTime() / 1_000;
}

function weekday(date) {
  return new Intl.DateTimeFormat("en-US", { weekday: "long" }).format(date);
}

function month(date) {
  return new Intl.DateTimeFormat("en-US", { month: "long" }).format(date);
}

// English output is asserted exactly: adding Korean must not restyle copy that
// already shipped. Korean is asserted on the parts that are stable across ICU
// versions — the day-period marker in particular is CLDR data that has changed
// between releases, and the app runs against the webview's ICU, not Node's.

test("formatShortMonthDayOrdinal formats month before ordinal day", () => {
  assert.equal(
    formatShortMonthDayOrdinal(localUnixSeconds(2026, 4, 19), "en"),
    "May 19th",
  );
});

test("formatShortMonthDayOrdinal handles ordinal suffixes", () => {
  const expectations = [
    [1, "May 1st"],
    [2, "May 2nd"],
    [3, "May 3rd"],
    [4, "May 4th"],
    [11, "May 11th"],
    [12, "May 12th"],
    [13, "May 13th"],
    [21, "May 21st"],
    [22, "May 22nd"],
    [23, "May 23rd"],
    [31, "May 31st"],
  ];

  for (const [day, expected] of expectations) {
    assert.equal(
      formatShortMonthDayOrdinal(localUnixSeconds(2026, 4, day), "en"),
      expected,
    );
  }
});

test("formatShortMonthDayOrdinal uses the Korean date form, not ordinals", () => {
  const formatted = formatShortMonthDayOrdinal(
    localUnixSeconds(2026, 4, 19),
    "ko",
  );

  assert.equal(formatted, "5월 19일");
  // "st/nd/rd/th" is an English-only construct; it must not survive into Korean.
  assert.doesNotMatch(formatted, /st|nd|rd|th/);
});

test("formatTimeWithoutDayPeriod drops the marker in both languages", () => {
  const afternoon = localUnixSeconds(2026, 4, 19, 14, 34);
  const morning = localUnixSeconds(2026, 4, 19, 8, 0);

  // Feeding each locale's own formatted output back in is the round trip the
  // callers actually perform — they hold the rendered string, not a timestamp.
  assert.equal(
    formatTimeWithoutDayPeriod(formatTime(afternoon, "en"), "en"),
    "2:34",
  );
  assert.equal(
    formatTimeWithoutDayPeriod(formatTime(morning, "en"), "en"),
    "8:00",
  );

  // Korean puts the marker in FRONT of the clock time, so the old
  // `/(?:AM|PM)$/` suffix regex left it untouched and `hideDayPeriod` did
  // nothing. Whatever marker the current CLDR uses, none of it may remain.
  assert.equal(
    formatTimeWithoutDayPeriod(formatTime(afternoon, "ko"), "ko"),
    "2:34",
  );
  assert.equal(
    formatTimeWithoutDayPeriod(formatTime(morning, "ko"), "ko"),
    "8:00",
  );
});

test("formatTimeWithoutDayPeriod leaves a 24-hour string untouched", () => {
  assert.equal(formatTimeWithoutDayPeriod("16:20", "en"), "16:20");
  assert.equal(formatTimeWithoutDayPeriod("16:20", "ko"), "16:20");
});

test("formatTime keeps the day-period marker", () => {
  const afternoon = localUnixSeconds(2026, 4, 19, 14, 34);

  assert.equal(formatTime(afternoon, "en"), "2:34 PM");
  assert.match(formatTime(afternoon, "ko"), /2:34/);
  assert.notEqual(formatTime(afternoon, "ko"), "2:34");
});

test("formatThreadSummaryLastReplyTime expands relative units", () => {
  const now = localUnixSeconds(2026, 4, 19);

  assert.equal(
    formatThreadSummaryLastReplyTime(now - 30, "en", now),
    "just now",
  );
  assert.equal(
    formatThreadSummaryLastReplyTime(now - 60, "en", now),
    "1 minute ago",
  );
  assert.equal(
    formatThreadSummaryLastReplyTime(now - 180, "en", now),
    "3 minutes ago",
  );
  assert.equal(
    formatThreadSummaryLastReplyTime(now - 3_600, "en", now),
    "1 hour ago",
  );
  assert.equal(
    formatThreadSummaryLastReplyTime(now - 10_800, "en", now),
    "3 hours ago",
  );
  assert.equal(
    formatThreadSummaryLastReplyTime(now - 86_400, "en", now),
    "1 day ago",
  );
  assert.equal(
    formatThreadSummaryLastReplyTime(now - 345_600, "en", now),
    "4 days ago",
  );
});

test("formatThreadSummaryLastReplyTime expands relative units in Korean", () => {
  const now = localUnixSeconds(2026, 4, 19);

  assert.equal(
    formatThreadSummaryLastReplyTime(now - 30, "ko", now),
    "방금 전",
  );
  // The old implementation appended an English "s" for anything but 1, so every
  // Korean unit above came out as "3 minutes ago".
  assert.equal(formatThreadSummaryLastReplyTime(now - 60, "ko", now), "1분 전");
  assert.equal(
    formatThreadSummaryLastReplyTime(now - 180, "ko", now),
    "3분 전",
  );
  assert.equal(
    formatThreadSummaryLastReplyTime(now - 10_800, "ko", now),
    "3시간 전",
  );
  assert.equal(
    formatThreadSummaryLastReplyTime(now - 345_600, "ko", now),
    "4일 전",
  );
});

test("formatThreadSummaryLastReplyTime uses ordinal dates for older replies", () => {
  const now = localUnixSeconds(2026, 5, 15);
  const replyAt = localUnixSeconds(2026, 4, 19);

  assert.equal(
    formatThreadSummaryLastReplyTime(replyAt, "en", now),
    "on May 19th",
  );
  // Korean drops the preposition rather than translating it.
  assert.equal(
    formatThreadSummaryLastReplyTime(replyAt, "ko", now),
    "5월 19일",
  );
});

test("formatDayHeading omits the year for current-year dates", () => {
  const now = new Date();
  const date = new Date(now.getFullYear(), (now.getMonth() + 6) % 12, 19, 12);

  assert.equal(
    formatDayHeading(date.getTime() / 1_000, "en"),
    `${weekday(date)}, ${month(date)} 19th`,
  );
});

test("formatDayHeading includes the year for other years", () => {
  const year = new Date().getFullYear() - 1;
  const date = new Date(year, 4, 19, 12);

  assert.equal(
    formatDayHeading(date.getTime() / 1_000, "en"),
    `${weekday(date)}, May 19th, ${year}`,
  );
});

test("formatDayHeading builds Korean dates year-first and omits ordinals", () => {
  const lastYear = new Date().getFullYear() - 1;
  const dated = new Date(lastYear, 4, 19, 12).getTime() / 1_000;

  const heading = formatDayHeading(dated, "ko");
  assert.match(heading, new RegExp(`^${lastYear}년 5월 19일`));
  assert.doesNotMatch(heading, /19th/);
});

test("formatDayHeading translates Today and Yesterday", () => {
  const now = Date.now() / 1_000;
  const yesterday = now - 86_400;

  assert.equal(formatDayHeading(now, "en"), "Today");
  assert.equal(formatDayHeading(now, "ko"), "오늘");
  assert.equal(formatDayHeading(yesterday, "en"), "Yesterday");
  assert.equal(formatDayHeading(yesterday, "ko"), "어제");
});

test("startOfLocalDaySeconds collapses a day's timestamps to one value", () => {
  const morning = new Date(2026, 5, 14, 8, 30, 15).getTime() / 1_000;
  const evening = new Date(2026, 5, 14, 23, 59, 59).getTime() / 1_000;
  const midnight = new Date(2026, 5, 14, 0, 0, 0).getTime() / 1_000;

  assert.equal(startOfLocalDaySeconds(morning), midnight);
  assert.equal(startOfLocalDaySeconds(evening), midnight);
});

test("startOfLocalDaySeconds separates adjacent calendar days", () => {
  const lateOn14 = new Date(2026, 5, 14, 23, 0, 0).getTime() / 1_000;
  const earlyOn15 = new Date(2026, 5, 15, 1, 0, 0).getTime() / 1_000;

  assert.notEqual(
    startOfLocalDaySeconds(lateOn14),
    startOfLocalDaySeconds(earlyOn15),
  );
});
