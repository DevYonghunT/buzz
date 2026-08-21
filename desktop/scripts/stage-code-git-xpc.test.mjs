import assert from "node:assert/strict";
import {
  chmodSync,
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  resolveBuildContext,
  stageXpcBundle,
  XPC_BUNDLE_IDENTIFIER,
  XPC_BUNDLE_NAME,
  XPC_EXECUTABLE_NAME,
} from "./stage-code-git-xpc.mjs";

function macEnvironment(overrides = {}) {
  return {
    TAURI_ENV_ARCH: "aarch64",
    TAURI_ENV_FAMILY: "unix",
    TAURI_ENV_PLATFORM: "darwin",
    TAURI_ENV_TARGET_TRIPLE: "aarch64-apple-darwin",
    ...overrides,
  };
}

function executable(path) {
  mkdirSync(join(path, ".."), { recursive: true });
  writeFileSync(path, "fixture");
  chmodSync(path, 0o755);
}

function withTemporaryDirectory(run) {
  const directory = mkdtempSync(join(tmpdir(), "schoolx-code-git-xpc-"));
  try {
    return run(directory);
  } finally {
    rmSync(directory, { force: true, recursive: true });
  }
}

test("resolves an omitted debug flag as the native arm64 release binary", () =>
  withTemporaryDirectory((tauriDir) => {
    const native = join(tauriDir, "target", "release", "buzz-desktop");
    const staleIntel = join(
      tauriDir,
      "target",
      "x86_64-apple-darwin",
      "release",
      "buzz-desktop",
    );
    executable(native);
    executable(staleIntel);

    const context = resolveBuildContext({
      env: macEnvironment(),
      tauriDir,
      readArchitectures(path) {
        return path === native ? ["arm64"] : ["x86_64"];
      },
    });
    assert.equal(context.binaryPath, native);
    assert.equal(context.profile, "release");
  }));

test("resolves an explicit Intel debug target directory", () =>
  withTemporaryDirectory((tauriDir) => {
    const binary = join(
      tauriDir,
      "target",
      "x86_64-apple-darwin",
      "debug",
      "buzz-desktop",
    );
    executable(binary);

    const context = resolveBuildContext({
      env: macEnvironment({
        SCHOOLX_CODE_GIT_CARGO_LAYOUT: "target-triple",
        TAURI_ENV_ARCH: "x86_64",
        TAURI_ENV_DEBUG: "true",
        TAURI_ENV_TARGET_TRIPLE: "x86_64-apple-darwin",
      }),
      tauriDir,
      readArchitectures: () => ["x86_64"],
    });
    assert.equal(context.binaryPath, binary);
    assert.equal(context.profile, "debug");
  }));

test("resolves an explicit Intel release target directory", () =>
  withTemporaryDirectory((tauriDir) => {
    const binary = join(
      tauriDir,
      "target",
      "x86_64-apple-darwin",
      "release",
      "buzz-desktop",
    );
    executable(binary);

    const context = resolveBuildContext({
      env: macEnvironment({
        SCHOOLX_CODE_GIT_CARGO_LAYOUT: "target-triple",
        TAURI_ENV_ARCH: "x86_64",
        TAURI_ENV_DEBUG: "false",
        TAURI_ENV_TARGET_TRIPLE: "x86_64-apple-darwin",
      }),
      tauriDir,
      readArchitectures: () => ["x86_64"],
    });
    assert.equal(context.binaryPath, binary);
    assert.equal(context.profile, "release");
  }));

test("fails closed when native and target-triple outputs are ambiguous", () =>
  withTemporaryDirectory((tauriDir) => {
    executable(join(tauriDir, "target", "release", "buzz-desktop"));
    executable(
      join(
        tauriDir,
        "target",
        "aarch64-apple-darwin",
        "release",
        "buzz-desktop",
      ),
    );

    assert.throws(
      () =>
        resolveBuildContext({
          env: macEnvironment(),
          tauriDir,
          readArchitectures: () => ["arm64"],
        }),
      /expected exactly one built.*found 2/s,
    );
  }));

test("rejects inconsistent target and hook architecture", () => {
  assert.throws(
    () =>
      resolveBuildContext({
        env: macEnvironment({ TAURI_ENV_ARCH: "x86_64" }),
        tauriDir: "/unused",
      }),
    /does not match target triple/,
  );
});

test("rejects invalid Tauri debug flag values", () => {
  assert.throws(
    () =>
      resolveBuildContext({
        env: macEnvironment({ TAURI_ENV_DEBUG: "" }),
        tauriDir: "/unused",
      }),
    /unexpected TAURI_ENV_DEBUG=""/,
  );
  assert.throws(
    () =>
      resolveBuildContext({
        env: macEnvironment({ TAURI_ENV_DEBUG: "0" }),
        tauriDir: "/unused",
      }),
    /unexpected TAURI_ENV_DEBUG="0"/,
  );
});

test("stages the renamed executable and fixed XPC Info.plist", () =>
  withTemporaryDirectory((directory) => {
    const source = join(directory, "buzz-desktop");
    const stagingRoot = join(directory, "generated", "code-git-xpc");
    executable(source);

    const bundle = stageXpcBundle({
      binaryPath: source,
      stagingRoot,
      version: "1.2.3-team.42",
    });
    assert.equal(bundle, join(stagingRoot, XPC_BUNDLE_NAME));
    assert.equal(
      readFileSync(
        join(bundle, "Contents", "MacOS", XPC_EXECUTABLE_NAME),
        "utf8",
      ),
      "fixture",
    );
    const plist = readFileSync(join(bundle, "Contents", "Info.plist"), "utf8");
    assert.match(
      plist,
      new RegExp(`<string>${XPC_BUNDLE_IDENTIFIER}</string>`),
    );
    assert.match(
      plist,
      /<key>CFBundlePackageType<\/key>\s*<string>XPC!<\/string>/,
    );
    assert.match(
      plist,
      /<key>CFBundleVersion<\/key>\s*<string>1\.2\.3<\/string>/,
    );
    assert.equal(existsSync(`${stagingRoot}.tmp-${process.pid}`), false);
  }));
