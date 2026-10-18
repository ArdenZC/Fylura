import { spawnSync } from "node:child_process";

export function buildPreparedTestArgs(testName, { ignored = true, testThreads } = {}) {
  return [
    testName,
    "--exact",
    ...(ignored ? ["--ignored"] : []),
    "--nocapture",
    ...(testThreads ? [`--test-threads=${testThreads}`] : []),
  ];
}

function requireSuccessfulProcess(result, context) {
  const stdout = result.stdout ? `\nstdout:\n${result.stdout}` : "";
  const stderr = result.stderr ? `\nstderr:\n${result.stderr}` : "";
  if (result.error) {
    throw new Error(`${context} failed to start: ${result.error.message}${stdout}${stderr}`);
  }
  if (result.signal) {
    throw new Error(`${context} terminated by signal ${result.signal}.${stdout}${stderr}`);
  }
  if (result.status !== 0) {
    throw new Error(`${context} failed with exit code ${result.status}.${stdout}${stderr}`);
  }
}

function listedTests(result) {
  return String(result.stdout ?? "")
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.endsWith(": test"))
    .map((line) => line.slice(0, -": test".length));
}

export function runPreparedTestBinary({
  executable,
  testName,
  ignored = true,
  testThreads,
  cwd,
  env,
  timeoutMs,
  spawnImpl = spawnSync,
  stdio = "inherit",
}) {
  const context = `Prepared performance binary "${executable}" test "${testName}"`;
  const options = {
    cwd,
    env,
    timeout: timeoutMs,
    windowsHide: true,
    encoding: "utf8",
  };
  const list = (ignoredOnly) => {
    const result = spawnImpl(executable, [
      testName, "--list", "--exact", ...(ignoredOnly ? ["--ignored"] : []),
    ], { ...options, stdio: "pipe" });
    requireSuccessfulProcess(result, `${context} listing (ignored=${ignoredOnly})`);
    return result;
  };
  const listing = list(ignored);
  const tests = listedTests(listing);
  if (tests.length !== 1 || tests[0] !== testName) {
    throw new Error(
      `${context} expected exactly one matching test (ignored=${ignored}); matched ${tests.length}: ${JSON.stringify(tests)}.`
      + `\nstdout:\n${listing.stdout ?? ""}\nstderr:\n${listing.stderr ?? ""}`,
    );
  }
  // --list without --ignored also lists ignored registrations. Such a test
  // would be silently skipped by the normal invocation, so reject it here.
  if (!ignored) {
    const ignoredListing = list(true);
    if (listedTests(ignoredListing).length !== 0) {
      throw new Error(
        `${context} requested a nonignored test but matched an ignored registration.`
        + `\nstdout:\n${ignoredListing.stdout ?? ""}\nstderr:\n${ignoredListing.stderr ?? ""}`,
      );
    }
    if (ignoredListing.stderr) process.stderr.write(ignoredListing.stderr);
  }
  if (listing.stderr) process.stderr.write(listing.stderr);
  console.log(`[perf-test-match] binary=${executable} test=${testName} ignored=${ignored} matched=1`);
  const args = buildPreparedTestArgs(testName, { ignored, testThreads });
  const result = spawnImpl(executable, args, { ...options, stdio });
  requireSuccessfulProcess(result, context);
  return result;
}
