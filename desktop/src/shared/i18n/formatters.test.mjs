import assert from "node:assert/strict";
import test from "node:test";

import {
  formatDateTime,
  formatNumber,
  formatRelativeTime,
  getDateTimeFormatter,
} from "./formatters.ts";

test("date formatting follows the selected app locale", () => {
  const date = new Date(Date.UTC(2026, 4, 19, 12));
  const options = {
    day: "numeric",
    month: "long",
    timeZone: "UTC",
    year: "numeric",
  };

  assert.equal(formatDateTime(date, "en", options), "May 19, 2026");
  assert.equal(formatDateTime(date, "ko", options), "2026년 5월 19일");
});

test("number and relative-time formatting use locale-specific Intl instances", () => {
  assert.match(
    formatNumber(1_234, "en", {
      currency: "USD",
      style: "currency",
    }),
    /^\$1,234\.00$/,
  );
  assert.match(
    formatNumber(1_234, "ko", {
      currency: "USD",
      style: "currency",
    }),
    /^US\$1,234\.00$/,
  );
  assert.equal(
    formatRelativeTime(-2, "day", "en", { numeric: "always" }),
    "2 days ago",
  );
  assert.equal(
    formatRelativeTime(-2, "day", "ko", { numeric: "always" }),
    "2일 전",
  );
});

test("formatter instances are cached by locale and options", () => {
  assert.equal(
    getDateTimeFormatter("ko", { dateStyle: "medium" }),
    getDateTimeFormatter("ko", { dateStyle: "medium" }),
  );
  assert.notEqual(
    getDateTimeFormatter("ko", { dateStyle: "medium" }),
    getDateTimeFormatter("en", { dateStyle: "medium" }),
  );
});
