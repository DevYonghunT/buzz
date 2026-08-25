import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const scriptsDir = dirname(fileURLToPath(import.meta.url));
const verifier = resolve(scriptsDir, "verify-macos-runtime-compatibility.sh");

function callBashFunction(name, { args = [], input = "" } = {}) {
  return spawnSync(
    "/bin/bash",
    [
      "-c",
      `source "$1"; shift; ${name} "$@"`,
      "runtime-verifier-test",
      verifier,
      ...args,
    ],
    { encoding: "utf8", input },
  );
}

function parseDeploymentTarget(input) {
  const result = callBashFunction("parse_macho_deployment_target", { input });
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.trim();
}

function assertDeploymentTargetRejected(input) {
  const result = callBashFunction("parse_macho_deployment_target", { input });
  assert.notEqual(result.status, 0, "malformed deployment target was accepted");
}

function classifySwiftDependency(dependency) {
  const result = callBashFunction("swift_dependency_kind", {
    args: [dependency],
  });
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.trim();
}

test("runtime parser accepts macOS LC_BUILD_VERSION platform 1", () => {
  assert.equal(
    parseDeploymentTarget(`fixture:
Load command 8
      cmd LC_BUILD_VERSION
  cmdsize 32
 platform 1
    minos 10.15
      sdk 27.0
`),
    "10.15",
  );
});

test("runtime parser treats LC_VERSION_MIN_MACOSX as macOS", () => {
  assert.equal(
    parseDeploymentTarget(`fixture:
Load command 7
      cmd LC_VERSION_MIN_MACOSX
  cmdsize 16
  version 10.15.0
      sdk 10.15.7
`),
    "10.15.0",
  );
});

test("runtime parser rejects missing, ambiguous, and non-macOS platforms", () => {
  assertDeploymentTargetRejected(`fixture:
Load command 8
      cmd LC_BUILD_VERSION
    minos 10.15
`);
  assertDeploymentTargetRejected(`fixture:
Load command 8
      cmd LC_BUILD_VERSION
 platform 1
 platform 1
    minos 10.15
`);
  assertDeploymentTargetRejected(`fixture:
Load command 8
      cmd LC_BUILD_VERSION
 platform 6
    minos 10.15
`);
  assertDeploymentTargetRejected(`fixture:
Load command 8
      cmd LC_BUILD_VERSION
 platform 1 trailing-data
    minos 10.15
`);
});

test("runtime parser rejects duplicate and conflicting deployment commands", () => {
  assertDeploymentTargetRejected(`fixture:
Load command 7
      cmd LC_BUILD_VERSION
 platform 1
    minos 10.15
Load command 8
      cmd LC_VERSION_MIN_MACOSX
  version 10.15
`);
  assertDeploymentTargetRejected(`fixture:
Load command 7
      cmd LC_BUILD_VERSION
 platform 1
    minos 10.15
Load command 8
      cmd LC_VERSION_MIN_MACOSX
  version 11.0
`);
});

test("otool dependency parser preserves the complete install name", () => {
  const input = `fixture:
\t/Applications/Xcode Beta.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/macosx/libswiftCore.dylib (compatibility version 0.0.0, current version 0.0.0)
\t/tmp/name (compatibility version fake)/libswiftCore.dylib (compatibility version 0.0.0, current version 0.0.0)
\t/usr/lib/swift/libswiftFoundation.dylib (compatibility version 1.0.0, current version 1.0.0, weak)
`;
  const result = callBashFunction("parse_otool_install_names", { input });
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.stdout.trim().split("\n"), [
    "/Applications/Xcode Beta.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/macosx/libswiftCore.dylib",
    "/tmp/name (compatibility version fake)/libswiftCore.dylib",
    "/usr/lib/swift/libswiftFoundation.dylib",
  ]);
});

test("system Swift rpath parser requires the complete exact path", () => {
  const exact = callBashFunction("system_swift_rpath_in_load_commands", {
    input: `fixture:
Load command 9
          cmd LC_RPATH
      cmdsize 32
         path /usr/lib/swift (offset 12)
`,
  });
  assert.equal(exact.status, 0, exact.stderr);

  for (const path of [
    "/usr/lib/swift fallback",
    "/Applications/Xcode Beta.app/usr/lib/swift",
  ]) {
    const result = callBashFunction("system_swift_rpath_in_load_commands", {
      input: `fixture:
Load command 9
          cmd LC_RPATH
      cmdsize 64
         path ${path} (offset 12)
`,
    });
    assert.notEqual(result.status, 0, `accepted non-system rpath ${path}`);
  }
});

test("Swift dependency policy accepts only one-component system install names", () => {
  assert.equal(
    classifySwiftDependency("/usr/lib/swift/libswiftCore.dylib"),
    "system",
  );
  assert.equal(
    classifySwiftDependency("@rpath/libswift_Concurrency.dylib"),
    "rpath",
  );
  for (const dependency of [
    "/Applications/Xcode Beta.app/Contents/Developer/usr/lib/swift/libswiftCore.dylib",
    "/usr/lib/swift/libswiftCore/../../Missing.dylib",
    "@rpath/libswiftCore/../Missing.dylib",
  ]) {
    assert.equal(classifySwiftDependency(dependency), "unsupported");
  }
});

test("otool dependency parser rejects malformed records", () => {
  const result = callBashFunction("parse_otool_install_names", {
    input: `fixture:
\t/usr/lib/swift/libswiftCore.dylib
`,
  });
  assert.notEqual(result.status, 0, "malformed otool -L output was accepted");
});
