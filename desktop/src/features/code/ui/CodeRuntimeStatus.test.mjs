import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { CodeRuntimeStatus } from "./CodeRuntimeStatus.tsx";

const replay = {
  status: "idle",
  subscriptionEpoch: null,
  request: null,
  needsAuthoritativeRefresh: false,
  approvalStateIncomplete: false,
};

function runtimeStatus(phase) {
  return {
    phase,
    generation: 0,
    executable: null,
    version: null,
    pid: null,
    userAgent: null,
    codexHome: null,
    platformFamily: null,
    platformOs: null,
    queuedNotifications: 0,
    lastError: null,
  };
}

function renderStatus({ pending = false, phase = "notInstalled" } = {}) {
  return renderToStaticMarkup(
    React.createElement(CodeRuntimeStatus, {
      error: null,
      onRefresh() {},
      onRetrySync() {},
      onStart() {},
      pending,
      replay,
      status: runtimeStatus(phase),
      subscriptionError: null,
    }),
  );
}

test("missing Codex offers an explicit installation re-check", () => {
  const html = renderStatus();

  assert.match(html, />Check again<\/button>/);
  assert.doesNotMatch(html, />Start<\/button>/);
});

test("installation re-check reports progress and prevents duplicate probes", () => {
  const html = renderStatus({ pending: true });

  assert.match(html, /<button[^>]*disabled=""[^>]*>[\s\S]*Checking…<\/button>/);
});

test("installed stopped runtime keeps Start and compact status refresh actions", () => {
  const html = renderStatus({ phase: "stopped" });

  assert.match(html, />Start<\/button>/);
  assert.match(html, /aria-label="Refresh Codex runtime status"/);
  assert.doesNotMatch(html, />Check again<\/button>/);
});
