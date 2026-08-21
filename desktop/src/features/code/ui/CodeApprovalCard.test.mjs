import assert from "node:assert/strict";
import test from "node:test";

import {
  codePermissionDisplayFromRequest,
  codePermissionDisplayRows,
  codePermissionResponse,
  isCodePermissionDisplayGrantable,
} from "./CodeApprovalCard.tsx";

const permissionDisplay = {
  grantable: true,
  network: { enabled: true },
  fileSystem: {
    entries: [
      {
        access: "write",
        path: { type: "path", path: "/project/generated" },
      },
      {
        access: "read",
        path: { type: "globPattern", pattern: "/project/**/*.rs" },
      },
      {
        access: "deny",
        path: {
          type: "special",
          value: { kind: "project_roots", subpath: ".git" },
        },
      },
      {
        access: "read",
        path: {
          type: "special",
          value: {
            kind: "unknown",
            path: "/external",
            subpath: "cache",
          },
        },
      },
    ],
    globScanMaxDepth: 12,
    read: ["/project/read"],
    write: ["/project/write"],
  },
};

test("permission card rows expose every nested display scope deterministically", () => {
  assert.deepEqual(codePermissionDisplayRows(permissionDisplay), [
    "Network: enabled",
    "Filesystem read path: /project/read",
    "Filesystem write path: /project/write",
    "Filesystem write path: /project/generated",
    "Filesystem read glob: /project/**/*.rs",
    "Filesystem deny special: project_roots/.git",
    "Filesystem read special: unknown: /external/cache",
    "Filesystem glob scan max depth: 12",
  ]);
});

test("permission card sends only opaque intent and lifetime", () => {
  const grant = codePermissionResponse("grant", "turn");
  const decline = codePermissionResponse("decline", "turn");
  assert.deepEqual(grant, {
    type: "permissions",
    intent: "grant",
    scope: "turn",
  });
  assert.deepEqual(decline, {
    type: "permissions",
    intent: "decline",
    scope: "turn",
  });
  assert.equal("permissions" in grant, false);
  assert.equal("strictAutoReview" in grant, false);
});

test("permission card fails closed for malformed, ungrantable, or raw requests", () => {
  assert.equal(codePermissionDisplayFromRequest({}), null);
  assert.equal(
    codePermissionDisplayFromRequest({
      permissions: { network: { enabled: true } },
      permissionDisplay,
    }),
    null,
  );
  assert.equal(isCodePermissionDisplayGrantable(null), false);
  assert.equal(
    isCodePermissionDisplayGrantable({
      ...permissionDisplay,
      grantable: false,
    }),
    false,
  );
  assert.equal(
    isCodePermissionDisplayGrantable({
      grantable: true,
      network: null,
      fileSystem: null,
    }),
    false,
  );
  assert.equal(
    isCodePermissionDisplayGrantable({
      ...permissionDisplay,
      fileSystem: {
        ...permissionDisplay.fileSystem,
        read: ["/project/[REDACTED]"],
      },
    }),
    false,
  );
  assert.equal(isCodePermissionDisplayGrantable(permissionDisplay), true);
});
