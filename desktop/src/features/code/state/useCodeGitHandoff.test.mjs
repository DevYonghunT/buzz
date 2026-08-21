import assert from "node:assert/strict";
import test from "node:test";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { codeSessionQueryKeys } from "./codeSessionQueries.ts";
import { useCodeGitHandoff } from "./useCodeGitHandoff.ts";

const scope = {
  communityId: "community-1",
  projectDtag: "project-1",
  repositoryIdentity: "a".repeat(64),
};
const attempt = {
  state: "unknown",
  operation: "commit",
  operationId: null,
  label: "commit",
  requestGeneration: 4,
  baselineStatusRevision: 10,
  receipt: null,
  message: "Commit outcome is unknown.",
};

function AttemptProbe({ runtimeGeneration }) {
  const handoff = useCodeGitHandoff({
    enabled: false,
    runtimeGeneration,
    scope,
    threadId: "thread-1",
  });
  return React.createElement(
    "output",
    null,
    handoff.attempt?.message ?? "none",
  );
}

test("Git handoff attempt survives remount and runtime generation changes", () => {
  const queryClient = new QueryClient();
  queryClient.setQueryData(
    codeSessionQueryKeys.threadGitAttempt({ scope, threadId: "thread-1" }),
    attempt,
  );

  try {
    const firstMount = renderToStaticMarkup(
      React.createElement(
        QueryClientProvider,
        { client: queryClient },
        React.createElement(AttemptProbe, { runtimeGeneration: 7 }),
      ),
    );
    const remountAfterRuntimeChange = renderToStaticMarkup(
      React.createElement(
        QueryClientProvider,
        { client: queryClient },
        React.createElement(AttemptProbe, { runtimeGeneration: 8 }),
      ),
    );

    assert.match(firstMount, /Commit outcome is unknown\./);
    assert.equal(remountAfterRuntimeChange, firstMount);
  } finally {
    queryClient.clear();
  }
});
