import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const workflow = readFileSync(".github/workflows/release-build.yml", "utf8").replace(/\r\n?/gu, "\n");

function step(name: string, nextName: string) {
  const start = workflow.indexOf(`      - name: ${name}\n`);
  expect(start).toBeGreaterThanOrEqual(0);
  const end = workflow.indexOf(`      - name: ${nextName}\n`, start + 1);
  expect(end).toBeGreaterThan(start);
  return workflow.slice(start, end);
}

function job(name: string, nextName?: string) {
  const start = workflow.indexOf(`  ${name}:\n`);
  expect(start).toBeGreaterThanOrEqual(0);
  const end = nextName ? workflow.indexOf(`  ${nextName}:\n`, start + 1) : workflow.length;
  expect(end).toBeGreaterThan(start);
  return workflow.slice(start, end < 0 ? workflow.length : end);
}

describe("release publication SBOM contract", () => {
  it("emits the source dependency SBOM pair only once across the platform matrix", () => {
    for (const [name, nextName] of [
      ["Generate Node SBOM", "Generate Rust SBOM"],
      ["Generate Rust SBOM", "Verify SBOM outputs"],
      ["Verify SBOM outputs", "Generate Windows checksums"],
    ] as const) {
      const source = step(name, nextName);
      expect(source).toContain("if: runner.os == 'Windows'");
    }

    expect(workflow).toContain("sbom-node.cdx.json");
    expect(workflow).toContain("src-tauri/sbom-rust.cdx.json");
  });

  it("keeps the tag publication verifier fail-closed on exactly one Node and one Rust SBOM", () => {
    const releaseVerification = step(
      "Verify final release artifacts and checksums",
      "Attach installers to GitHub Release",
    );

    expect(releaseVerification).toContain("Get-ChildItem -LiteralPath $root -Recurse -Filter *.cdx.json -File");
    expect(releaseVerification).toContain(
      'if ($sboms.Count -ne 2) { throw "Expected Node and Rust SBOMs; found $($sboms.Count)." }',
    );
    expect(releaseVerification).toContain('$document.bomFormat -ne "CycloneDX"');
  });

  it("still uploads the SBOM pair with platform installers and publishes it from downloaded artifacts", () => {
    const upload = step("Upload installers", "Checkout release commit");
    expect(upload).toContain("sbom-node.cdx.json");
    expect(upload).toContain("src-tauri/sbom-rust.cdx.json");
    expect(upload).toContain("installers-*.sha256");

    const publish = workflow.slice(workflow.indexOf("      - name: Attach installers to GitHub Release\n"));
    expect(publish).toContain("release-artifacts/**/*.cdx.json");
    expect(publish).toContain("release-artifacts/**/*.sha256");
  });
});

describe("release workflow security and W6 publication contract", () => {
  it("keeps workflow and build/qualification jobs least-privileged", () => {
    const beforeJobs = workflow.slice(0, workflow.indexOf("jobs:\n"));
    const qualification = job("release-qualified-validation", "build");
    const build = job("build", "release");
    const release = job("release");

    expect(beforeJobs).toContain("permissions:\n  contents: read\n");
    expect(beforeJobs).not.toContain("contents: write");
    expect(qualification).toContain("permissions:\n      actions: read\n      contents: read\n");
    expect(build).toContain("permissions:\n      contents: read\n");
    expect(build).not.toContain("contents: write");
    expect(release).toContain("permissions:\n      contents: write\n");
    expect(workflow.match(/^\s+contents: write\s*$/gmu)).toHaveLength(1);
  });

  it("does not persist checkout credentials outside a publication need", () => {
    for (const [name, nextName] of [
      ["Checkout release qualification verifier", "Setup Node"],
      ["Checkout", "Verify Apple Silicon runner"],
      ["Checkout release commit", "Download release artifacts"],
    ] as const) {
      expect(step(name, nextName)).toContain("persist-credentials: false");
    }
    expect(workflow.match(/persist-credentials: false/g)).toHaveLength(3);
  });

  it("proves a plain v* tag push is not publication authorization during W6", () => {
    const triggers = workflow.slice(0, workflow.indexOf("permissions:\n"));
    const release = job("release");
    const deferral = step("Enforce W6 publication deferral", "Checkout release commit");

    expect(triggers).toContain("on:\n  workflow_dispatch:\n");
    expect(triggers).not.toMatch(/^\s+push:\s*$/mu);
    expect(workflow).toContain('W6_RELEASE_PUBLICATION_ENABLED: "false"');
    expect(release).toContain(
      "if: ${{ inputs.qa_only != true && github.event_name == 'workflow_dispatch' && startsWith(github.ref, 'refs/tags/') }}",
    );
    expect(deferral).toContain('$env:W6_RELEASE_PUBLICATION_ENABLED -ne "true"');
    expect(deferral).toContain("GitHub Release publication is disabled while W6 product maturity work is active.");
    expect(release.indexOf("Enforce W6 publication deferral")).toBeLessThan(
      release.indexOf("Attach installers to GitHub Release"),
    );
  });

  it("keeps QA packaging explicit, read-only, short-lived, and separate from release qualification", () => {
    const triggers = workflow.slice(0, workflow.indexOf("permissions:\n"));
    const qualification = job("release-qualified-validation", "build");
    const build = job("build", "release");
    const release = job("release");
    const qaUpload = step("Upload QA-only unqualified installers", "Upload installers");

    expect(triggers).toContain("qa_only:");
    expect(triggers).toContain("default: false");
    expect(triggers).toContain("type: boolean");
    expect(qualification).toContain("if: ${{ inputs.qa_only != true }}");
    expect(build).toContain("needs: release-qualified-validation");
    expect(build).toContain(
      "if: ${{ always() && ((inputs.qa_only == true && startsWith(github.ref, 'refs/heads/')) || (inputs.qa_only != true && needs.release-qualified-validation.result == 'success')) }}",
    );
    expect(build).toContain(
      "include: ${{ fromJSON(inputs.qa_only == true && '[{\"name\":\"Windows NSIS\",\"os\":\"windows-latest\"}]' || '[{\"name\":\"Windows NSIS\",\"os\":\"windows-latest\"},{\"name\":\"macOS DMG\",\"os\":\"macos-latest\"}]') }}",
    );
    expect(build).toContain("permissions:\n      contents: read\n");
    expect(qaUpload).toContain("if: ${{ inputs.qa_only == true }}");
    expect(qaUpload).toContain("name: Fylura-QA-UNQUALIFIED-");
    expect(qaUpload).toContain("-CANDIDATE-");
    expect(qaUpload).toContain("retention-days: 5");
    expect(qaUpload).toContain("qa-package-provenance.json");
    expect(qaUpload).toContain("installers-*.sha256");
    expect(release).toContain("inputs.qa_only != true");
    expect(release).toContain("permissions:\n      contents: write\n");
  });

  it("records exact source and verifies the built Windows x64 / Apple Silicon package architecture", () => {
    expect(workflow).toContain("node scripts/qaPackageProvenance.mjs verify-architecture");
    expect(workflow).toContain("node scripts/qaPackageProvenance.mjs write-provenance");
    expect(workflow).toContain("EXPECTED_SHA: ${{ github.sha }}");
    expect(workflow).toContain("EXPECTED_REF: ${{ github.ref }}");
    expect(workflow).toContain("QA_RUNNER_OS: ${{ runner.os }}");
    expect(workflow).toContain("QA_RUNNER_ARCH: ${{ runner.arch }}");
    expect(workflow).toContain("run: npm run build -- --no-sign");
    expect(workflow).toContain("Verify unsigned macOS DMG");
  });
});

