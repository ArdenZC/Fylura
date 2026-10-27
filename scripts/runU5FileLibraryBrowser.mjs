/** One repeatable U5 browser-mock run over the existing U3 and U4 journeys. */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";

const output = path.resolve(process.env.U5_OUTPUT_DIR ?? ".tmp-tests/u5-browser");
const u3Output = path.join(output, "u3");
const u4Output = path.join(output, "u4");
await mkdir(output, { recursive: true });

for (const [script, variable, directory] of [
  ["scripts/runU3FileLibraryBrowser.mjs", "U3_OUTPUT_DIR", u3Output],
  ["scripts/runU4FileLibraryBrowser.mjs", "U4_OUTPUT_DIR", u4Output]
]) {
  const result = spawnSync(process.execPath, [script], {
    cwd: process.cwd(),
    env: { ...process.env, [variable]: directory },
    stdio: "inherit"
  });
  assert.equal(result.error, undefined, `${script} did not start`);
  assert.equal(result.status, 0, `${script} exited with ${result.status}`);
}

const u3 = JSON.parse(await readFile(path.join(u3Output, "checks.json"), "utf8"));
const u4 = JSON.parse(await readFile(path.join(u4Output, "checks.json"), "utf8"));
const manifest = {
  evidenceClass: "browser mock; Chromium renderer with deterministic mocked data and simulated Win32 navigator",
  nativeWindowsGui: "UNVERIFIED by this script",
  nativeBackendFilesystem: "UNVERIFIED by this script",
  macOS: "UNVERIFIED",
  viewportCoverage: ["820x800", "980x800", "1440x900", "1600x1000"],
  u3: { checks: u3.checks, errors: u3.errors, artifacts: "u3/" },
  u4: { checks: u4.checks, errors: u4.errors, artifacts: "u4/" }
};
assert.deepEqual(manifest.u3.errors, []);
assert.deepEqual(manifest.u4.errors, []);
await writeFile(path.join(output, "checks.json"), JSON.stringify(manifest, null, 2));
console.log(JSON.stringify({ result: "PASS", output, evidenceClass: manifest.evidenceClass, checks: u3.checks.length + u4.checks.length }, null, 2));
