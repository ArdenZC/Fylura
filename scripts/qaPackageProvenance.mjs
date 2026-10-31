import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import process from "node:process";
import { fileURLToPath } from "node:url";

const architectureReportPath = ".qa-package-architecture.json";

function fail(message) {
  throw new Error(message);
}

function run(command, args, cwd) {
  const result = spawnSync(command, args, {
    cwd,
    encoding: "utf8",
    windowsHide: true,
  });
  if (result.error || result.status !== 0) {
    fail(
      `Command failed: ${command} ${args.join(" ")}\n${result.stdout ?? ""}\n${result.stderr ?? ""}`,
    );
  }
  return (result.stdout ?? "").trim();
}

function gitValue(root, revision) {
  return run("git", ["rev-parse", revision], root);
}

function packageFilesFor(root, runnerOs) {
  const directory = runnerOs === "Windows"
    ? path.join(root, "src-tauri", "target", "release", "bundle", "nsis")
    : path.join(root, "src-tauri", "target", "release", "bundle", "dmg");
  const extension = runnerOs === "Windows" ? ".exe" : ".dmg";
  if (!fs.existsSync(directory)) fail(`Package directory is missing: ${directory}`);
  const files = fs.readdirSync(directory, { withFileTypes: true })
    .filter((entry) => entry.isFile() && path.extname(entry.name).toLowerCase() === extension)
    .map((entry) => path.join(directory, entry.name))
    .sort();
  if (files.length === 0) fail(`No ${extension} package was produced in ${directory}`);
  return files;
}

function decodeChecksumFile(filePath) {
  const buffer = fs.readFileSync(filePath);
  if (buffer.length >= 2 && buffer[0] === 0xff && buffer[1] === 0xfe) {
    return buffer.subarray(2).toString("utf16le");
  }
  return buffer.toString("utf8").replace(/^\uFEFF/u, "");
}

export function verifyInstallerChecksums(files, contents) {
  const entries = new Map();
  for (const line of contents.split(/\r?\n/u).map((value) => value.trim()).filter(Boolean)) {
    const match = line.match(/^([a-f\d]{64})\s+\*?(.+?)\s*$/iu);
    if (!match) fail(`Malformed SHA-256 checksum entry: ${line}`);
    const filename = match[2].trim();
    if (entries.has(filename)) fail(`Duplicate checksum entry for ${filename}`);
    entries.set(filename, match[1].toLowerCase());
  }

  const expectedNames = new Set(files.map((filePath) => path.basename(filePath)));
  if (entries.size !== expectedNames.size || [...entries.keys()].some((name) => !expectedNames.has(name))) {
    fail("Checksum manifest does not cover exactly the produced installer files.");
  }

  return files.map((filePath) => {
    const filename = path.basename(filePath);
    const bytes = fs.readFileSync(filePath);
    if (bytes.length === 0) fail(`Installer is empty: ${filename}`);
    const sha256 = crypto.createHash("sha256").update(bytes).digest("hex");
    if (entries.get(filename) !== sha256) fail(`Checksum mismatch for ${filename}`);
    return { filename, sizeBytes: bytes.length, sha256 };
  });
}

function requireVersionedPackages(files, version, runnerOs) {
  for (const filePath of files) {
    const filename = path.basename(filePath);
    if (!filename.includes(version)) fail(`Installer version mismatch: ${filename}`);
    if (runnerOs === "macOS" && (/x86_64|universal/iu).test(filename)) {
      fail(`macOS package name is not Apple-Silicon-only: ${filename}`);
    }
  }
}

function readRustHostTarget(root) {
  const version = run("rustc", ["-vV"], root);
  const target = version.match(/^host:\s*(\S+)$/mu)?.[1];
  if (!target) fail("rustc -vV did not report a host target.");
  return target;
}

function verifyWindowsX64Application(root, runnerArch, rustTarget) {
  if (runnerArch !== "X64" || process.arch !== "x64" || rustTarget !== "x86_64-pc-windows-msvc") {
    fail(
      `Windows QA packaging requires an x64 MSVC runner and Rust target; got runner=${runnerArch}, node=${process.arch}, rust=${rustTarget}.`,
    );
  }
  const relativePath = path.join("src-tauri", "target", "release", "fylura.exe");
  const executablePath = path.join(root, relativePath);
  if (!fs.existsSync(executablePath)) fail(`Built Windows application is missing: ${relativePath}`);
  const bytes = fs.readFileSync(executablePath);
  if (bytes.length < 0x40 || bytes.toString("ascii", 0, 2) !== "MZ") {
    fail(`Built Windows application is not a valid PE executable: ${relativePath}`);
  }
  const peOffset = bytes.readUInt32LE(0x3c);
  if (peOffset + 6 > bytes.length || bytes.toString("ascii", peOffset, peOffset + 4) !== "PE\0\0") {
    fail(`Built Windows application has no valid PE header: ${relativePath}`);
  }
  const machine = bytes.readUInt16LE(peOffset + 4);
  if (machine !== 0x8664) {
    fail(`Built Windows application is not x64 (PE machine 0x${machine.toString(16)}).`);
  }
  return {
    path: relativePath.split(path.sep).join("/"),
    architectures: ["x64"],
    peMachine: "0x8664",
  };
}

function findAppBundles(root, depth = 0) {
  if (depth > 4) return [];
  const bundles = [];
  for (const entry of fs.readdirSync(root, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    const entryPath = path.join(root, entry.name);
    if (entry.name.endsWith(".app")) {
      bundles.push(entryPath);
    } else {
      bundles.push(...findAppBundles(entryPath, depth + 1));
    }
  }
  return bundles;
}

function verifyAppleSiliconDmg(root, dmgPath, version) {
  const temporaryRoot = fs.mkdtempSync(path.join(os.tmpdir(), "fylura-qa-dmg-"));
  const mountPath = path.join(temporaryRoot, "volume");
  fs.mkdirSync(mountPath);
  let mounted = false;
  try {
    run("hdiutil", ["attach", "-readonly", "-nobrowse", "-mountpoint", mountPath, "-quiet", dmgPath], root);
    mounted = true;
    const bundles = findAppBundles(mountPath);
    if (bundles.length === 0) fail(`No .app bundle was found inside ${path.basename(dmgPath)}`);

    return bundles.map((bundlePath) => {
      const plistPath = path.join(bundlePath, "Contents", "Info.plist");
      const executableName = run("/usr/libexec/PlistBuddy", ["-c", "Print :CFBundleExecutable", plistPath], root);
      const bundleVersion = run("/usr/libexec/PlistBuddy", ["-c", "Print :CFBundleShortVersionString", plistPath], root);
      if (bundleVersion !== version) {
        fail(`DMG app version mismatch in ${path.basename(dmgPath)}: ${bundleVersion}`);
      }
      const executableRelativePath = path.join("Contents", "MacOS", executableName);
      const executablePath = path.join(bundlePath, executableRelativePath);
      if (!fs.existsSync(executablePath)) {
        fail(`DMG app executable is missing: ${path.basename(bundlePath)}/${executableRelativePath}`);
      }
      const architectures = run("lipo", ["-archs", executablePath], root).split(/\s+/u).filter(Boolean).sort();
      if (architectures.length !== 1 || architectures[0] !== "arm64") {
        fail(`DMG app is not arm64-only: ${path.basename(bundlePath)} has ${architectures.join(", ")}`);
      }
      return {
        dmg: path.basename(dmgPath),
        bundle: path.basename(bundlePath),
        version: bundleVersion,
        architectures,
      };
    });
  } finally {
    if (mounted) run("hdiutil", ["detach", "-quiet", mountPath], root);
    fs.rmSync(temporaryRoot, { recursive: true, force: true });
  }
}

function verifyArchitecture(root, env) {
  const runnerOs = env.QA_RUNNER_OS;
  const runnerArch = env.QA_RUNNER_ARCH;
  if (!["Windows", "macOS"].includes(runnerOs)) fail(`Unsupported QA package runner: ${runnerOs ?? "<missing>"}`);
  const rustTarget = readRustHostTarget(root);
  const version = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8")).version;
  const packages = packageFilesFor(root, runnerOs);
  requireVersionedPackages(packages, version, runnerOs);

  let application;
  if (runnerOs === "Windows") {
    application = verifyWindowsX64Application(root, runnerArch, rustTarget);
  } else {
    if (runnerArch !== "ARM64" || process.arch !== "arm64" || rustTarget !== "aarch64-apple-darwin") {
      fail(
        `macOS QA packaging requires an Apple-Silicon runner and Rust target; got runner=${runnerArch}, node=${process.arch}, rust=${rustTarget}.`,
      );
    }
    application = packages.flatMap((dmgPath) => verifyAppleSiliconDmg(root, dmgPath, version));
  }

  const report = {
    runnerOs,
    runnerArchitecture: runnerArch,
    nodeArchitecture: process.arch,
    rustTarget,
    application,
  };
  fs.writeFileSync(path.join(root, architectureReportPath), `${JSON.stringify(report, null, 2)}\n`, "utf8");
  console.log(`QA package architecture verified: ${runnerOs} ${runnerArch} / ${rustTarget}`);
}

function requireString(value, label) {
  if (typeof value !== "string" || value.trim() === "") fail(`${label} is required.`);
  return value.trim();
}

export function buildQaPackageProvenance({
  sourceSha,
  sourceTree,
  repository,
  ref,
  runnerOs,
  runnerArchitecture,
  runId,
  runAttempt,
  version,
  artifactType,
  architecture,
  artifacts,
  checksumManifest,
}) {
  if (!/^[a-f\d]{40}$/iu.test(sourceSha)) fail("QA provenance requires a full source commit SHA.");
  if (!/^[a-f\d]{40}$/iu.test(sourceTree)) fail("QA provenance requires the exact source tree SHA.");
  if (!Array.isArray(artifacts) || artifacts.length === 0) {
    fail("QA provenance requires at least one installer artifact.");
  }
  const expectedArtifactType = runnerOs === "Windows" ? "windows-nsis" : "macos-unsigned-dmg";
  if (artifactType !== expectedArtifactType) fail(`Unexpected QA artifact type for ${runnerOs}: ${artifactType}`);
  return {
    schemaVersion: 1,
    classification: "QA_ONLY_UNQUALIFIED",
    qualification: {
      releaseQualified: false,
      installedGuiAcceptance: "not claimed",
      packageSigning: runnerOs === "macOS" ? "Build requested --no-sign; no release signing or notarization claimed" : "Unsigned QA build; no Authenticode claim",
    },
    source: {
      repository: requireString(repository, "Repository"),
      sha: sourceSha.toLowerCase(),
      tree: sourceTree.toLowerCase(),
      ref: requireString(ref, "Source ref"),
    },
    workflow: {
      file: ".github/workflows/release-build.yml",
      event: "workflow_dispatch",
      runId: requireString(String(runId ?? ""), "Workflow run ID"),
      runAttempt: requireString(String(runAttempt ?? ""), "Workflow run attempt"),
    },
    build: {
      runner: {
        platform: requireString(runnerOs, "Runner platform"),
        architecture: requireString(runnerArchitecture, "Runner architecture"),
      },
      application: architecture,
    },
    package: {
      type: artifactType,
      version: requireString(version, "Package version"),
      checksumManifest: requireString(checksumManifest, "Checksum manifest"),
      artifacts,
    },
  };
}

function writeProvenance(root, env) {
  if (env.QA_ONLY_PACKAGE_MODE !== "true") fail("QA provenance may only be written in explicit QA-only mode.");
  if (env.QA_EVENT_NAME !== "workflow_dispatch") fail("QA packaging requires a manual workflow_dispatch event.");
  const expectedSha = requireString(env.EXPECTED_SHA, "Expected source SHA").toLowerCase();
  const sourceSha = gitValue(root, "HEAD").toLowerCase();
  const sourceTree = gitValue(root, "HEAD^{tree}").toLowerCase();
  if (sourceSha !== expectedSha) fail(`Checked out source SHA ${sourceSha} does not match workflow SHA ${expectedSha}.`);

  const runnerOs = requireString(env.QA_RUNNER_OS, "Runner platform");
  const runnerArchitecture = requireString(env.QA_RUNNER_ARCH, "Runner architecture");
  const architecture = JSON.parse(fs.readFileSync(path.join(root, architectureReportPath), "utf8"));
  if (architecture.runnerOs !== runnerOs || architecture.runnerArchitecture !== runnerArchitecture) {
    fail("Verified package architecture report does not match the current runner.");
  }
  if (runnerOs === "Windows") {
    if (
      architecture.rustTarget !== "x86_64-pc-windows-msvc"
      || architecture.application?.architectures?.length !== 1
      || architecture.application.architectures[0] !== "x64"
    ) {
      fail("QA provenance requires a verified Windows x64 application binary.");
    }
  } else if (
    runnerOs !== "macOS"
    || architecture.rustTarget !== "aarch64-apple-darwin"
    || !Array.isArray(architecture.application)
    || architecture.application.length === 0
    || architecture.application.some((bundle) =>
      bundle.architectures?.length !== 1 || bundle.architectures[0] !== "arm64",
    )
  ) {
    fail("QA provenance requires a verified arm64-only macOS application bundle.");
  }
  const expectedArtifactType = runnerOs === "Windows" ? "windows-nsis" : "macos-unsigned-dmg";
  if (runnerOs !== "Windows" && runnerOs !== "macOS") fail(`Unsupported QA runner platform: ${runnerOs}`);
  const version = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8")).version;
  const packageFiles = packageFilesFor(root, runnerOs);
  requireVersionedPackages(packageFiles, version, runnerOs);
  const checksumManifest = runnerOs === "Windows" ? "installers-windows.sha256" : "installers-macos.sha256";
  const checksumPath = path.join(root, checksumManifest);
  if (!fs.existsSync(checksumPath)) fail(`Checksum manifest is missing: ${checksumManifest}`);
  const artifacts = verifyInstallerChecksums(packageFiles, decodeChecksumFile(checksumPath));
  const manifest = buildQaPackageProvenance({
    sourceSha,
    sourceTree,
    repository: env.REPOSITORY,
    ref: env.EXPECTED_REF,
    runnerOs,
    runnerArchitecture,
    runId: env.QA_RUN_ID,
    runAttempt: env.QA_RUN_ATTEMPT,
    version,
    artifactType: expectedArtifactType,
    architecture,
    artifacts,
    checksumManifest,
  });
  fs.writeFileSync(
    path.join(root, "qa-package-provenance.json"),
    `${JSON.stringify(manifest, null, 2)}\n`,
    "utf8",
  );
  console.log(`Wrote QA-only provenance for ${sourceSha} / ${sourceTree}`);
}

function main() {
  const command = process.argv[2];
  const root = process.cwd();
  if (command === "verify-architecture") {
    verifyArchitecture(root, process.env);
    return;
  }
  if (command === "write-provenance") {
    writeProvenance(root, process.env);
    return;
  }
  fail(`Unknown QA package provenance command: ${command ?? "<missing>"}`);
}

const directPath = process.argv[1] ? path.resolve(process.argv[1]) : "";
if (directPath && directPath === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
