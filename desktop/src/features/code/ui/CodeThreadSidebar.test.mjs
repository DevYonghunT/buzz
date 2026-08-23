import assert from "node:assert/strict";
import test from "node:test";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { codeSessionQueryKeys } from "../state/codeSessionQueries.ts";
import { CodeThreadSidebar } from "./CodeThreadSidebar.tsx";

const scope = {
  communityId: "community-1",
  projectDtag: "project-1",
  repositoryIdentity: "a".repeat(64),
};

function renderSidebar({ creating = false, loading = false, refreshReady }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  queryClient.setQueryData(codeSessionQueryKeys.worktrees(scope), []);

  try {
    return renderToStaticMarkup(
      React.createElement(
        QueryClientProvider,
        { client: queryClient },
        React.createElement(CodeThreadSidebar, {
          actionPendingId: null,
          actionsReady: false,
          canCreate: false,
          creating,
          isForkBlocked: () => false,
          isLifecycleBlocked: () => false,
          loading,
          async onArchiveThread() {},
          onCreate() {},
          async onForkThread() {},
          onOpenPreparation() {},
          onRefresh() {},
          async onRenameThread() {},
          onSelectThread() {},
          async onUnarchiveThread() {},
          preparations: [],
          refreshReady,
          scope,
          selectedThreadId: null,
          threads: [],
        }),
      ),
    );
  } finally {
    queryClient.clear();
  }
}

function refreshButtonTag(html) {
  const match = html.match(/<button[^>]*aria-label="Refresh Code tasks"[^>]*>/);
  assert.ok(match, "Refresh Code tasks button should render");
  return match[0];
}

test("task-list errors keep Refresh available without unlocking task actions", () => {
  const html = renderSidebar({ refreshReady: true });

  assert.doesNotMatch(refreshButtonTag(html), /disabled=""/);
  assert.match(html, /<button[^>]*disabled=""[^>]*>[\s\S]*?New task<\/button>/);
});

test("loading and mutation gates still disable task-list Refresh", () => {
  assert.match(
    refreshButtonTag(renderSidebar({ loading: true, refreshReady: true })),
    /disabled=""/,
  );
  assert.match(
    refreshButtonTag(renderSidebar({ creating: true, refreshReady: true })),
    /disabled=""/,
  );
  assert.match(
    refreshButtonTag(renderSidebar({ refreshReady: false })),
    /disabled=""/,
  );
});
