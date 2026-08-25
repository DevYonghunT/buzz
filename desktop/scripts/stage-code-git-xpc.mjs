import {
  constants,
  chmodSync,
  copyFileSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { spawnSync } from "node:child_process";

export const XPC_BUNDLE_IDENTIFIER =
  "io.github.schoolx520.app.schoolx-code-git";
export const XPC_BUNDLE_NAME = `${XPC_BUNDLE_IDENTIFIER}.xpc`;
export const XPC_EXECUTABLE_NAME = "schoolx-code-git";

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const DESKTOP_DIR = resolve(SCRIPT_DIR, "..");
const TAURI_DIR = join(DESKTOP_DIR, "src-tauri");
const STAGING_ROOT = join(TAURI_DIR, "generated", "code-git-xpc");

function requiredEnvironment(env, name) {
  const value = env[name];
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`Tauri did not provide required hook environment ${name}`);
  }
  return value;
}

function profileFromEnvironment(env) {
  // Tauri CLI 2.11 only defines TAURI_ENV_DEBUG for debug builds. Release
  // hooks omit it, despite the config schema documentation describing a
  // literal "false" value. Keep accepting "false" for normalized callers.
  const debug = env.TAURI_ENV_DEBUG;
  if (debug === undefined || debug === "false") return "release";
  if (debug === "true") return "debug";
  throw new Error(`unexpected TAURI_ENV_DEBUG=${JSON.stringify(debug)}`);
}

function cargoTargetDirectory(env, tauriDir) {
  if (!env.CARGO_TARGET_DIR) return join(tauriDir, "target");
  return isAbsolute(env.CARGO_TARGET_DIR)
    ? resolve(env.CARGO_TARGET_DIR)
    : resolve(tauriDir, env.CARGO_TARGET_DIR);
}

function candidateBinaryPaths(env, tauriDir, target, profile) {
  const targetDir = cargoTargetDirectory(env, tauriDir);
  const native = join(targetDir, profile, "buzz-desktop");
  const targetTriple = join(targetDir, target, profile, "buzz-desktop");
  const layout = env.SCHOOLX_CODE_GIT_CARGO_LAYOUT;

  if (layout === undefined) return [native, targetTriple];
  if (layout === "native") return [native];
  if (layout === "target-triple") return [targetTriple];
  throw new Error(
    `unexpected SCHOOLX_CODE_GIT_CARGO_LAYOUT=${JSON.stringify(layout)}`,
  );
}

function isRegularExecutable(path) {
  try {
    const stat = lstatSync(path);
    return stat.isFile() && !stat.isSymbolicLink() && (stat.mode & 0o111) !== 0;
  } catch (error) {
    if (error?.code === "ENOENT") return false;
    throw error;
  }
}

function readMachOArchitectures(path) {
  const result = spawnSync("/usr/bin/lipo", ["-archs", path], {
    encoding: "utf8",
    env: { PATH: "/usr/bin:/bin" },
  });
  if (result.error) {
    throw new Error(
      `could not inspect ${path} with lipo: ${result.error.message}`,
    );
  }
  if (result.status !== 0) {
    throw new Error(
      `lipo rejected ${path}: ${(result.stderr || result.stdout).trim()}`,
    );
  }
  const architectures = result.stdout.trim().split(/\s+/).filter(Boolean);
  if (architectures.length === 0) {
    throw new Error(`lipo reported no architecture for ${path}`);
  }
  return architectures;
}

function canonicalMacOSVersion(value) {
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)(?:\.(0|[1-9]\d*))?$/.exec(value);
  if (!match) {
    throw new Error(`invalid macOS deployment target ${JSON.stringify(value)}`);
  }
  const patch = Number(match[3] ?? "0");
  return `${Number(match[1])}.${Number(match[2])}${patch === 0 ? "" : `.${patch}`}`;
}

export function parseMacOSDeploymentTarget(loadCommands) {
  const records = [];
  const commandBlocks = loadCommands
    .split(/^\s*Load command \d+\s*$/m)
    .slice(1);

  for (const block of commandBlocks) {
    const commandMatches = [
      ...block.matchAll(
        /^\s*cmd\s+(LC_BUILD_VERSION|LC_VERSION_MIN_MACOSX)\s*$/gm,
      ),
    ];
    if (commandMatches.length === 0) continue;
    if (commandMatches.length !== 1) {
      throw new Error("otool reported an ambiguous macOS deployment command");
    }

    const command = commandMatches[0][1];
    if (command === "LC_BUILD_VERSION") {
      const platformMatches = [...block.matchAll(/^\s*platform\s+(\S+)\s*$/gm)];
      if (platformMatches.length === 0) {
        throw new Error("LC_BUILD_VERSION did not report platform");
      }
      if (platformMatches.length !== 1) {
        throw new Error("LC_BUILD_VERSION reported ambiguous platform values");
      }
      if (platformMatches[0][1] !== "1") {
        throw new Error(
          `LC_BUILD_VERSION must target macOS platform 1; found ${platformMatches[0][1]}`,
        );
      }
    }
    const field = command === "LC_BUILD_VERSION" ? "minos" : "version";
    const valueMatches = [
      ...block.matchAll(new RegExp(`^\\s*${field}\\s+(\\S+)\\s*$`, "gm")),
    ];
    if (valueMatches.length === 0) {
      throw new Error(`${command} did not report ${field}`);
    }
    if (valueMatches.length !== 1) {
      throw new Error(`${command} reported ambiguous ${field} values`);
    }
    records.push({
      command,
      version: canonicalMacOSVersion(valueMatches[0][1]),
    });
  }

  if (records.length === 0) {
    throw new Error("otool reported no macOS deployment target");
  }
  if (records.length !== 1) {
    const versions = new Set(records.map(({ version }) => version));
    if (versions.size !== 1) {
      throw new Error(
        `otool reported conflicting macOS deployment targets: ${records
          .map(({ command, version }) => `${command}=${version}`)
          .join(", ")}`,
      );
    }
    throw new Error(
      `otool reported an ambiguous macOS deployment target ${records[0].version} in ${records.length} load commands`,
    );
  }

  return records[0].version;
}

function readMachODeploymentTarget(path) {
  const result = spawnSync("/usr/bin/otool", ["-l", path], {
    encoding: "utf8",
    env: { PATH: "/usr/bin:/bin" },
  });
  if (result.error) {
    throw new Error(
      `could not inspect ${path} with otool: ${result.error.message}`,
    );
  }
  if (result.status !== 0) {
    throw new Error(
      `otool rejected ${path}: ${(result.stderr || result.stdout).trim()}`,
    );
  }
  return parseMacOSDeploymentTarget(result.stdout);
}

export function resolveBuildContext({
  env,
  tauriDir,
  readArchitectures = readMachOArchitectures,
  readDeploymentTarget = readMachODeploymentTarget,
}) {
  const target = requiredEnvironment(env, "TAURI_ENV_TARGET_TRIPLE");
  const platform = requiredEnvironment(env, "TAURI_ENV_PLATFORM");
  const arch = requiredEnvironment(env, "TAURI_ENV_ARCH");
  const family = requiredEnvironment(env, "TAURI_ENV_FAMILY");

  if (platform !== "darwin") return null;
  if (family !== "unix") {
    throw new Error(`macOS XPC staging requires TAURI_ENV_FAMILY=unix`);
  }

  const architectureByTarget = new Map([
    ["aarch64-apple-darwin", ["aarch64", "arm64", "11.0"]],
    ["x86_64-apple-darwin", ["x86_64", "x86_64", "10.15"]],
  ]);
  const expected = architectureByTarget.get(target);
  if (!expected) {
    throw new Error(`unsupported macOS target triple ${target}`);
  }
  const [expectedHookArch, expectedMachOArch, expectedDeploymentTarget] =
    expected;
  if (arch !== expectedHookArch) {
    throw new Error(
      `Tauri hook arch ${arch} does not match target triple ${target}`,
    );
  }

  const profile = profileFromEnvironment(env);
  const candidates = candidateBinaryPaths(env, tauriDir, target, profile);
  const matching = [];
  const rejected = [];
  for (const path of candidates) {
    if (!isRegularExecutable(path)) {
      rejected.push(`${path} (missing or not a regular executable)`);
      continue;
    }
    const architectures = readArchitectures(path);
    if (architectures.length !== 1 || architectures[0] !== expectedMachOArch) {
      rejected.push(
        `${path} (Mach-O architectures: ${architectures.join(",")})`,
      );
      continue;
    }
    matching.push(path);
  }

  if (matching.length !== 1) {
    const detail = [
      ...matching.map((path) => `${path} (matching)`),
      ...rejected,
    ];
    throw new Error(
      `expected exactly one built ${target}/${profile} buzz-desktop binary; found ${matching.length}\n${detail.join("\n")}`,
    );
  }

  const deploymentTarget = canonicalMacOSVersion(
    readDeploymentTarget(matching[0]),
  );
  if (deploymentTarget !== expectedDeploymentTarget) {
    throw new Error(
      `${target} SchoolX Code Git XPC must target macOS ${expectedDeploymentTarget}; ${matching[0]} targets macOS ${deploymentTarget}`,
    );
  }

  return { binaryPath: matching[0], deploymentTarget, profile, target };
}

function xpcInfoPlist(version) {
  const match = /^(\d+)\.(\d+)\.(\d+)(?:-[0-9A-Za-z.-]+)?$/.exec(version);
  if (!match)
    throw new Error(`desktop package version is not semver: ${version}`);
  const bundleVersion = `${match[1]}.${match[2]}.${match[3]}`;
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleExecutable</key>
    <string>${XPC_EXECUTABLE_NAME}</string>
    <key>CFBundleIdentifier</key>
    <string>${XPC_BUNDLE_IDENTIFIER}</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>SchoolX Code Git</string>
    <key>CFBundlePackageType</key>
    <string>XPC!</string>
    <key>CFBundleShortVersionString</key>
    <string>${bundleVersion}</string>
    <key>CFBundleVersion</key>
    <string>${bundleVersion}</string>
    <key>LSMinimumSystemVersion</key>
    <string>10.15</string>
    <key>XPCService</key>
    <dict>
        <key>ServiceType</key>
        <string>Application</string>
    </dict>
</dict>
</plist>
`;
}

export function stageXpcBundle({ binaryPath, stagingRoot, version }) {
  const temporaryRoot = `${stagingRoot}.tmp-${process.pid}`;
  const bundleRoot = join(temporaryRoot, XPC_BUNDLE_NAME);
  const contents = join(bundleRoot, "Contents");
  const macos = join(contents, "MacOS");
  const executable = join(macos, XPC_EXECUTABLE_NAME);

  rmSync(temporaryRoot, { force: true, recursive: true });
  mkdirSync(macos, { recursive: true });
  copyFileSync(binaryPath, executable, constants.COPYFILE_EXCL);
  // copyFile preserves normal Cargo output permissions, but force the XPC
  // entrypoint executable in case a prior filesystem strips them.
  const sourceMode = lstatSync(binaryPath).mode & 0o777;
  const executableMode = sourceMode | 0o111;
  // Node's chmod API is intentionally used instead of spawning ambient tools.
  chmodSync(executable, executableMode);
  writeFileSync(join(contents, "Info.plist"), xpcInfoPlist(version), {
    encoding: "utf8",
    flag: "wx",
  });

  rmSync(stagingRoot, { force: true, recursive: true });
  renameSync(temporaryRoot, stagingRoot);
  return join(stagingRoot, XPC_BUNDLE_NAME);
}

function main() {
  const context = resolveBuildContext({
    env: process.env,
    tauriDir: TAURI_DIR,
  });
  if (context === null) {
    console.log("Skipping SchoolX Code Git XPC staging on non-macOS target");
    return;
  }

  const packageJson = JSON.parse(
    readFileSync(join(DESKTOP_DIR, "package.json"), "utf8"),
  );
  const bundlePath = stageXpcBundle({
    binaryPath: context.binaryPath,
    stagingRoot: STAGING_ROOT,
    version: packageJson.version,
  });
  console.log(
    `Staged ${context.target}/${context.profile} SchoolX Code Git XPC at ${bundlePath}`,
  );
  // Unsigned team builds intentionally contain the same bundle shape. Their
  // empty TeamIdentifier makes the runtime peer-signing gate return
  // unsupported before any Code Git mutation is attempted.
}

const entrypoint = process.argv[1]
  ? pathToFileURL(resolve(process.argv[1])).href
  : undefined;
if (entrypoint === import.meta.url) {
  try {
    main();
  } catch (error) {
    console.error(`SchoolX Code Git XPC staging failed: ${error.message}`);
    process.exitCode = 1;
  }
}
