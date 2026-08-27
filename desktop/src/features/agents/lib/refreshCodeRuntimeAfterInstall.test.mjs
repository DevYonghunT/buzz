import assert from "node:assert/strict";
import test from "node:test";

import {
  refreshCodeRuntimeAfterInstall,
  shouldRefreshCodeRuntimeAfterInstall,
} from "./refreshCodeRuntimeAfterInstall.ts";

test("only a successful Codex installer result requests a Code refresh", () => {
  assert.equal(
    shouldRefreshCodeRuntimeAfterInstall("codex", { success: true }, null),
    true,
  );
  assert.equal(
    shouldRefreshCodeRuntimeAfterInstall("codex", { success: false }, null),
    false,
  );
  assert.equal(
    shouldRefreshCodeRuntimeAfterInstall(
      "codex",
      { success: true },
      new Error("installer failed"),
    ),
    false,
  );
  assert.equal(
    shouldRefreshCodeRuntimeAfterInstall(
      "claude-code",
      { success: true },
      null,
    ),
    false,
  );
});

test("Codex install re-probes Code before refreshing runtime status", async () => {
  const calls = [];
  const probe = {
    available: true,
    executable: "C:\\Codex\\codex.exe",
    version: "codex-cli 0.149.1",
    error: null,
  };
  const queryClient = {
    setQueryData(key, value) {
      calls.push(["set", key, value]);
    },
    async invalidateQueries(options) {
      calls.push(["invalidate", options.queryKey]);
    },
  };

  await refreshCodeRuntimeAfterInstall("codex", queryClient, {
    async probeCodeRuntime() {
      calls.push(["probe"]);
      return probe;
    },
  });

  assert.deepEqual(calls, [
    ["probe"],
    ["set", ["schoolx-code", "runtime", "probe"], probe],
    ["invalidate", ["schoolx-code", "runtime", "status"]],
  ]);
});

test("non-Codex installs leave SchoolX Code untouched", async () => {
  let calls = 0;
  await refreshCodeRuntimeAfterInstall(
    "claude-code",
    {
      setQueryData() {
        calls += 1;
      },
      async invalidateQueries() {
        calls += 1;
      },
    },
    {
      async probeCodeRuntime() {
        calls += 1;
        throw new Error("must not probe");
      },
    },
  );
  assert.equal(calls, 0);
});
