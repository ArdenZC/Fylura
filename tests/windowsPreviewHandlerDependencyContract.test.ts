import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const repositoryRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const read = (relativePath: string) =>
  fs.readFileSync(path.join(repositoryRoot, relativePath), "utf8");
const buildScript = read("scripts/buildWindowsPreviewHandler.mjs");
const verifierScript = read(
  "scripts/verifyWindowsPreviewHandlerDependencies.mjs",
);
const releaseWorkflow = read(".github/workflows/release-build.yml");

describe("Windows Preview Handler dependency contract", () => {
  it("isolates static CRT to the production Preview Handler cargo invocation", () => {
    expect(buildScript).toContain('"-C target-feature=+crt-static"');
    expect(buildScript).toContain("CARGO_ENCODED_RUSTFLAGS");
    expect(buildScript).toContain("CARGO_TARGET_DIR: nativeTargetDirectory");
    expect(buildScript).not.toContain("process.env.RUSTFLAGS =");
  });

  it("requires an x64 COM DLL and rejects redistributable runtime imports", () => {
    expect(verifierScript).toContain(
      'expectedFileName = "fylura_windows_preview_handler.dll"',
    );
    expect(verifierScript).toContain("inspection.machine !== 0x8664");
    expect(verifierScript).toContain('"DllGetClassObject", "DllCanUnloadNow"');
    expect(verifierScript).toContain("disallowedRuntimeImport");
    expect(verifierScript).toContain("api-ms-win-crt-");
  });

  it("runs the artifact-specific dependency check after Windows packaging", () => {
    const packageStep = releaseWorkflow.indexOf(
      "- name: Package Windows installer",
    );
    const dependencyStep = releaseWorkflow.indexOf(
      "- name: Verify Windows Preview Handler dependency contract",
    );
    const uploadStep = releaseWorkflow.indexOf("- name: Upload installers");

    expect(packageStep).toBeGreaterThanOrEqual(0);
    expect(dependencyStep).toBeGreaterThan(packageStep);
    expect(uploadStep).toBeGreaterThan(dependencyStep);
    expect(releaseWorkflow.slice(dependencyStep, uploadStep)).toContain(
      "node scripts/verifyWindowsPreviewHandlerDependencies.mjs",
    );
  });
});
