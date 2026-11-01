import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  buildQaPackageProvenance,
  verifyInstallerChecksums,
} from "../scripts/qaPackageProvenance.mjs";
import {
  QA_FILENAME_CANDIDATE_FEATURE,
  resolveTauriBuildFeatures,
} from "../scripts/tauriBuildFeatures.mjs";

let temporaryRoot = "";

beforeEach(() => {
  temporaryRoot = fs.mkdtempSync(path.join(os.tmpdir(), "fylura-qa-package-contract-"));
});

afterEach(() => {
  if (temporaryRoot) fs.rmSync(temporaryRoot, { recursive: true, force: true });
});

describe("QA package provenance", () => {
  it("keeps production builds candidate-free and includes the candidate only in QA dispatch packages", () => {
    expect(resolveTauriBuildFeatures({})).toEqual(["desktop-runtime"]);
    expect(resolveTauriBuildFeatures({ QA_ONLY_PACKAGE_MODE: "false", GITHUB_EVENT_NAME: "workflow_dispatch" }))
      .toEqual(["desktop-runtime"]);
    expect(resolveTauriBuildFeatures({ QA_ONLY_PACKAGE_MODE: "true", GITHUB_EVENT_NAME: "workflow_dispatch" }))
      .toEqual(["desktop-runtime", QA_FILENAME_CANDIDATE_FEATURE]);
    expect(() => resolveTauriBuildFeatures({ QA_ONLY_PACKAGE_MODE: "true", GITHUB_EVENT_NAME: "push" }))
      .toThrow("explicit QA workflow dispatch");
  });

  it("requires the checksum manifest to cover exactly the produced installers", () => {
    const installerPath = path.join(temporaryRoot, "Fylura_0.1.40_x64-setup.exe");
    fs.writeFileSync(installerPath, "synthetic NSIS installer bytes");
    const digest = crypto.createHash("sha256").update(fs.readFileSync(installerPath)).digest("hex");

    expect(verifyInstallerChecksums([installerPath], `${digest}  ${path.basename(installerPath)}\n`)).toEqual([
      {
        filename: path.basename(installerPath),
        sizeBytes: fs.statSync(installerPath).size,
        sha256: digest,
      },
    ]);
    expect(() => verifyInstallerChecksums([installerPath], `${"0".repeat(64)}  ${path.basename(installerPath)}\n`))
      .toThrow("Checksum mismatch");
    expect(() => verifyInstallerChecksums([installerPath], "")).toThrow(
      "Checksum manifest does not cover exactly",
    );
  });

  it("labels the exact source and artifact as QA-only and unqualified", () => {
    const manifest = buildQaPackageProvenance({
      sourceSha: "a".repeat(40),
      sourceTree: "b".repeat(40),
      repository: "ArdenZC/Fylura",
      ref: "refs/heads/codex/issue-494-qa-packaging",
      runnerOs: "Windows",
      runnerArchitecture: "X64",
      runId: "41234567890",
      runAttempt: "1",
      version: "0.1.40",
      artifactType: "windows-nsis",
      cargoFeatures: ["desktop-runtime", QA_FILENAME_CANDIDATE_FEATURE],
      architecture: {
        runnerOs: "Windows",
        runnerArchitecture: "X64",
        nodeArchitecture: "x64",
        rustTarget: "x86_64-pc-windows-msvc",
        application: { path: "src-tauri/target/release/fylura.exe", architectures: ["x64"] },
      },
      artifacts: [{
        filename: "Fylura_0.1.40_x64-setup.exe",
        sizeBytes: 100,
        sha256: "c".repeat(64),
      }],
      checksumManifest: "installers-windows.sha256",
    });

    expect(manifest.classification).toBe("QA_ONLY_UNQUALIFIED");
    expect(manifest.qualification.releaseQualified).toBe(false);
    expect(manifest.source).toMatchObject({
      repository: "ArdenZC/Fylura",
      sha: "a".repeat(40),
      tree: "b".repeat(40),
    });
    expect(manifest.build.runner).toEqual({ platform: "Windows", architecture: "X64" });
    expect(manifest.build.cargoFeatures).toEqual(["desktop-runtime", QA_FILENAME_CANDIDATE_FEATURE]);
    expect(manifest.build.filenameCandidate).toEqual({
      compiled: true,
      runtimeOptInVariable: "ZC_GLOBAL_SEARCH_FILENAME_RELEVANCE_CANDIDATE",
      defaultState: "off",
      enableValue: "1",
    });
    expect(manifest.package.artifacts[0]?.sha256).toBe("c".repeat(64));
  });
});

