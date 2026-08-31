#!/usr/bin/env node

import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  readdirSync,
  rmSync,
  statSync,
  utimesSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

function fail(message) {
  throw new Error(message);
}

function parseArguments(argv) {
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const name = argv[index];
    const value = argv[index + 1];
    if (!name?.startsWith("--") || value === undefined) {
      fail(
        "usage: refresh-schoolx-codex-schema.mjs --baseline <version> --version <version> --codex <absolute-path>",
      );
    }
    values.set(name.slice(2), value);
  }
  const baseline = values.get("baseline");
  const version = values.get("version");
  const codex = values.get("codex");
  if (!baseline || !version || !codex) {
    fail(
      "usage: refresh-schoolx-codex-schema.mjs --baseline <version> --version <version> --codex <absolute-path>",
    );
  }
  if (
    !/^0\.[0-9]+\.[0-9]+$/.test(baseline) ||
    !/^0\.[0-9]+\.[0-9]+$/.test(version)
  ) {
    fail("baseline and version must be exact numeric Codex 0.x.y releases");
  }
  if (!isAbsolute(codex)) {
    fail("--codex must be an absolute path");
  }
  return { baseline, version, codex };
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function collectJsonPaths(root, directory = root) {
  const paths = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      paths.push(...collectJsonPaths(root, path));
    } else if (entry.isFile() && entry.name.endsWith(".json")) {
      paths.push(relative(root, path).split(sep).join("/"));
    }
  }
  return paths.sort();
}

function canonicalSchema(path) {
  return execFileSync("jq", ["-S", "-c", ".", path]);
}

function aggregate(entries) {
  return sha256(
    Buffer.from(entries.map(([hash, path]) => `${hash}  ${path}\n`).join("")),
  );
}

function required(schema) {
  return new Set(Array.isArray(schema.required) ? schema.required : []);
}

function properties(schema) {
  return new Set(Object.keys(schema.properties ?? {}));
}

function difference(left, right) {
  return [...left].filter((value) => !right.has(value)).sort();
}

function setEquals(left, right) {
  return (
    left.size === right.size && [...left].every((value) => right.has(value))
  );
}

function structuralValue(value) {
  if (Array.isArray(value)) {
    return value.map(structuralValue);
  }
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, nested]) => [key, structuralValue(nested)]),
    );
  }
  return value;
}

function structurallyEqual(left, right) {
  return (
    JSON.stringify(structuralValue(JSON.parse(left))) ===
    JSON.stringify(structuralValue(JSON.parse(right)))
  );
}

function replaceFixtureVersion(value, baseline, version) {
  if (Array.isArray(value)) {
    return value.map((nested) =>
      replaceFixtureVersion(nested, baseline, version),
    );
  }
  if (value && typeof value === "object") {
    const replaced = Object.fromEntries(
      Object.entries(value).map(([key, nested]) => [
        key,
        replaceFixtureVersion(nested, baseline, version),
      ]),
    );
    if (Object.hasOwn(replaced, "cliVersion")) {
      const usesPaginatedHistory = version === "0.151.0";
      replaced.historyMode = usesPaginatedHistory ? "paginated" : "legacy";
      if (usesPaginatedHistory && replaced.source === "appServer") {
        replaced.source = "vscode";
      }
    }
    return replaced;
  }
  if (value === baseline) {
    return version;
  }
  if (value === `codex-cli ${baseline}`) {
    return `codex-cli ${version}`;
  }
  return value;
}

const { baseline, version, codex } = parseArguments(process.argv.slice(2));
const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, "..");
const desktopRoot = join(repositoryRoot, "desktop");
const fixtures = join(
  repositoryRoot,
  "desktop/src-tauri/src/code_workspace/fixtures",
);
const baselineManifestPath = join(
  fixtures,
  `codex-${baseline}-schema-manifest.json`,
);
const baselineArchivePath = join(
  fixtures,
  `codex-${baseline}-selected-schemas.tar.gz.base64`,
);
const baselineWirePath = join(fixtures, `codex-${baseline}-wire.json`);
const nextManifestPath = join(
  fixtures,
  `codex-${version}-schema-manifest.json`,
);
const nextArchivePath = join(
  fixtures,
  `codex-${version}-selected-schemas.tar.gz.base64`,
);
const nextWirePath = join(fixtures, `codex-${version}-wire.json`);
const expectedVersion = `codex-cli ${version}`;
const actualVersion = execFileSync(codex, ["--version"], {
  encoding: "utf8",
}).trim();
if (actualVersion !== expectedVersion) {
  fail(`expected ${expectedVersion}, got ${actualVersion}`);
}

const temporaryRoot = mkdtempSync(join(tmpdir(), `schoolx-codex-${version}-`));
try {
  const generatedRoot = join(temporaryRoot, "generated");
  const selectedRoot = join(temporaryRoot, "selected");
  const baselineRoot = join(temporaryRoot, "baseline");
  const compressedArchive = join(temporaryRoot, "selected.tar.gz");
  const uncompressedArchive = join(temporaryRoot, "selected.tar");
  mkdirSync(generatedRoot);
  mkdirSync(selectedRoot);
  mkdirSync(baselineRoot);

  execFileSync(
    codex,
    ["app-server", "generate-json-schema", "--out", generatedRoot],
    { stdio: "inherit" },
  );

  const baselineManifest = JSON.parse(
    readFileSync(baselineManifestPath, "utf8"),
  );
  const baselineCompressed = Buffer.from(
    readFileSync(baselineArchivePath, "utf8").replaceAll(/\s/g, ""),
    "base64",
  );
  const baselineCompressedPath = join(temporaryRoot, "baseline.tar.gz");
  writeFileSync(baselineCompressedPath, baselineCompressed);
  execFileSync("tar", ["-xzf", baselineCompressedPath, "-C", baselineRoot]);

  const generatedPaths = collectJsonPaths(generatedRoot);
  const fullEntries = generatedPaths.map((path) => [
    sha256(canonicalSchema(join(generatedRoot, path))),
    path,
  ]);
  const addedMethods = {};
  const turnsListSchemas = [
    "v2/ThreadTurnsListParams.json",
    "v2/ThreadTurnsListResponse.json",
  ];
  if (turnsListSchemas.every((path) => generatedPaths.includes(path))) {
    addedMethods["thread/turns/list"] = turnsListSchemas;
  }
  const baselineSelectedPaths = new Set(
    baselineManifest.schemas.map((entry) => entry[1]),
  );
  const selectedPaths = [
    ...new Set([
      ...baselineSelectedPaths,
      ...Object.values(addedMethods).flat(),
    ]),
  ].sort();
  const selectedEntries = [];
  let exactUnchanged = 0;
  let structurallyUnchanged = 0;
  for (const path of selectedPaths) {
    const generatedPath = join(generatedRoot, path);
    if (!statSync(generatedPath).isFile()) {
      fail(`generated schema is missing ${path}`);
    }
    const canonical = canonicalSchema(generatedPath);
    if (baselineSelectedPaths.has(path)) {
      const baselineCanonical = readFileSync(join(baselineRoot, path));
      if (canonical.equals(baselineCanonical)) {
        exactUnchanged += 1;
      }
      if (structurallyEqual(baselineCanonical, canonical)) {
        structurallyUnchanged += 1;
      }
    }
    const outputPath = join(selectedRoot, path);
    mkdirSync(dirname(outputPath), { recursive: true });
    writeFileSync(outputPath, canonical);
    chmodSync(outputPath, 0o644);
    utimesSync(outputPath, new Date(0), new Date(0));
    selectedEntries.push([sha256(canonical), path]);
  }

  const requestPropertiesRemoved = [];
  const requestPropertiesAdded = {};
  const requestRequiredFieldsChanged = [];
  const responseRequiredFieldsChanged = [];
  for (const [method, [requestPath, responsePath]] of Object.entries(
    baselineManifest.methods,
  )) {
    const oldRequest = JSON.parse(
      readFileSync(join(baselineRoot, requestPath)),
    );
    const nextRequest = JSON.parse(
      readFileSync(join(selectedRoot, requestPath)),
    );
    const removed = difference(properties(oldRequest), properties(nextRequest));
    if (removed.length > 0) {
      requestPropertiesRemoved.push({ method, properties: removed });
    }
    const added = difference(properties(nextRequest), properties(oldRequest));
    if (added.length > 0) {
      requestPropertiesAdded[method] = added;
    }
    if (!setEquals(required(oldRequest), required(nextRequest))) {
      requestRequiredFieldsChanged.push(method);
    }

    const oldResponse = JSON.parse(
      readFileSync(join(baselineRoot, responsePath)),
    );
    const nextResponse = JSON.parse(
      readFileSync(join(selectedRoot, responsePath)),
    );
    if (!setEquals(required(oldResponse), required(nextResponse))) {
      responseRequiredFieldsChanged.push(method);
    }
  }
  if (
    requestPropertiesRemoved.length > 0 ||
    requestRequiredFieldsChanged.length > 0 ||
    responseRequiredFieldsChanged.length > 0
  ) {
    fail(
      `breaking SchoolX request/response drift: ${JSON.stringify({
        requestPropertiesRemoved,
        requestRequiredFieldsChanged,
        responseRequiredFieldsChanged,
      })}`,
    );
  }

  execFileSync(
    "tar",
    [
      "-cf",
      uncompressedArchive,
      "--format",
      "ustar",
      "--uid",
      "0",
      "--gid",
      "0",
      "--uname",
      "root",
      "--gname",
      "root",
      "-C",
      selectedRoot,
      ...selectedPaths,
    ],
    { env: { ...process.env, COPYFILE_DISABLE: "1" } },
  );
  writeFileSync(
    compressedArchive,
    execFileSync("gzip", ["-n", "-9", "-c", uncompressedArchive]),
  );
  const compressedArchiveBytes = readFileSync(compressedArchive);
  const encodedArchive = compressedArchiveBytes.toString("base64");
  writeFileSync(nextArchivePath, `${encodedArchive}\n`);

  const dispatchSchemas = new Set(baselineManifest.dispatchSchemas);
  const leafEntries = selectedEntries.filter(
    ([, path]) => !dispatchSchemas.has(path),
  );
  const executable = realpathSync(codex);
  const nextManifest = structuredClone(baselineManifest);
  nextManifest.source = {
    ...nextManifest.source,
    cliVersion: expectedVersion,
    generatedFileCount: generatedPaths.length,
    fullGeneratedSetSha256: aggregate(fullEntries),
    selectedLeafSchemasSha256: aggregate(leafEntries),
    selectedSchemaCount: selectedEntries.length,
    selectedSchemasSha256: aggregate(selectedEntries),
    selectedSchemaArtifact: `codex-${version}-selected-schemas.tar.gz.base64`,
    selectedSchemaArtifactSha256: sha256(compressedArchiveBytes),
    executableSha256: sha256(readFileSync(executable)),
  };
  nextManifest.runtimeVersionRequirement = `codex-cli ${version
    .split(".")
    .slice(0, 2)
    .join(".")}.<numeric patch>`;
  nextManifest.provenSnapshotVersion = expectedVersion;
  nextManifest.methods = {
    ...nextManifest.methods,
    ...addedMethods,
  };
  nextManifest.schemas = selectedEntries;
  const shapePaths = new Set([
    ...Object.values(nextManifest.methods).flat(),
    ...Object.values(nextManifest.notifications),
    ...Object.values(nextManifest.serverRequests).flat(),
  ]);
  nextManifest.schemaShapes = Object.fromEntries(
    [...shapePaths]
      .sort()
      .map((path) => {
        const schema = JSON.parse(readFileSync(join(selectedRoot, path)));
        return [
          path,
          {
            required: [...required(schema)].sort(),
            properties: [...properties(schema)].sort(),
          },
        ];
      }),
  );
  nextManifest.compatibilityWithBaseline = {
    baselineVersion: `codex-cli ${baseline}`,
    retainedSelectedSchemaPaths: baselineSelectedPaths.size,
    addedSelectedSchemaPaths: difference(
      new Set(selectedPaths),
      baselineSelectedPaths,
    ),
    schoolxMethodsAdded: Object.keys(addedMethods).sort(),
    exactUnchangedSelectedSchemas: exactUnchanged,
    exactChangedSelectedSchemas: selectedEntries.length - exactUnchanged,
    structurallyUnchangedSelectedSchemas: structurallyUnchanged,
    structurallyChangedSelectedSchemas:
      selectedEntries.length - structurallyUnchanged,
    numericRepresentationOnlyChanges: structurallyUnchanged - exactUnchanged,
    schoolxRequestPropertiesRemoved: requestPropertiesRemoved,
    schoolxRequestPropertiesAdded: requestPropertiesAdded,
    schoolxRequestRequiredFieldsChanged: requestRequiredFieldsChanged,
    schoolxResponseRequiredFieldsChanged: responseRequiredFieldsChanged,
  };
  writeFileSync(nextManifestPath, `${JSON.stringify(nextManifest, null, 2)}\n`);

  const nextWire = replaceFixtureVersion(
    JSON.parse(readFileSync(baselineWirePath, "utf8")),
    baseline,
    version,
  );
  const commandApproval = nextWire.approvals.find(
    ({ request }) => request.method === "item/commandExecution/requestApproval",
  );
  if (!commandApproval) {
    fail("baseline wire fixture has no command execution approval");
  }
  const resumeTurns = structuredClone(nextWire.threadResume.result.thread.turns);
  commandApproval.request.params.kind = "command";
  nextWire.threadResume.params.excludeTurns = true;
  nextWire.threadResume.result.thread.turns = [];
  nextWire.threadResume.result.itemsBackwardsCursor =
    "items-backwards-0-151";
  nextWire.threadResume.result.turnsBackwardsCursor =
    "turns-backwards-0-151";
  if (addedMethods["thread/turns/list"]) {
    nextWire.threadTurnsList = {
      method: "thread/turns/list",
      params: {
        threadId: nextWire.threadResume.params.threadId,
        cursor: nextWire.threadResume.result.turnsBackwardsCursor,
        limit: 100,
        sortDirection: "desc",
        itemsView: "full",
      },
      result: {
        data: resumeTurns.map((turn) => ({
          ...turn,
          itemsView: "full",
        })),
        nextCursor: null,
        backwardsCursor: "turns-forward-0-151",
      },
    };
  }
  writeFileSync(nextWirePath, `${JSON.stringify(nextWire, null, 2)}\n`);
  execFileSync(
    "pnpm",
    [
      "exec",
      "biome",
      "format",
      "--write",
      relative(desktopRoot, nextManifestPath),
      relative(desktopRoot, nextWirePath),
    ],
    { cwd: desktopRoot, stdio: "inherit" },
  );

  console.log(
    JSON.stringify(
      {
        manifest: relative(repositoryRoot, nextManifestPath),
        archive: relative(repositoryRoot, nextArchivePath),
        wire: relative(repositoryRoot, nextWirePath),
        generatedSchemas: generatedPaths.length,
        selectedSchemas: selectedEntries.length,
        exactUnchanged,
        structurallyUnchanged,
      },
      null,
      2,
    ),
  );
} finally {
  rmSync(temporaryRoot, { recursive: true, force: true });
}
