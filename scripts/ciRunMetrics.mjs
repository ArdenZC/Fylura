import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const CACHE_LABELS = ["rust-cache", "prepared-binary-cache", "fixture-cache"];

export function parseCacheLog(log) {
  const observed = {};
  for (const match of String(log).matchAll(/\[perf-prepare\]\s+(rust-cache|prepared-binary-cache|fixture-cache)=(hit|miss)\b/g)) {
    observed[match[1]] = match[2];
  }
  return observed;
}

function jobMinutes(job) {
  const start = Date.parse(job.startedAt ?? "");
  const end = Date.parse(job.completedAt ?? "");
  if (!Number.isFinite(start) || !Number.isFinite(end) || end < start) return null;
  return (end - start) / 60_000;
}

/** Run-level conclusions and per-job durations are reported separately. */
export function summarizeCiRuns(runs) {
  const outcomes = { success: 0, failure: 0, cancelled: 0, other: 0 };
  const slowJobs = [];
  const failures = new Map();
  for (const run of runs) {
    const result = String(run.conclusion || run.status || "").toLowerCase();
    if (result === "success") outcomes.success += 1;
    else if (result === "failure") outcomes.failure += 1;
    else if (result === "cancelled") outcomes.cancelled += 1;
    else outcomes.other += 1;
    for (const job of run.jobs ?? []) {
      const minutes = jobMinutes(job);
      if (minutes !== null) slowJobs.push({ name: job.name ?? "unnamed", minutes, runId: run.databaseId });
      if (["failure", "timed_out"].includes(job.conclusion)) {
        failures.set(job.name ?? "unnamed", (failures.get(job.name ?? "unnamed") ?? 0) + 1);
      }
    }
  }
  slowJobs.sort((a, b) => b.minutes - a.minutes);
  return { total: runs.length, outcomes, slowJobs: slowJobs.slice(0, 8),
    failures: [...failures].sort((a, b) => b[1] - a[1]) };
}

export function formatCiMetrics(summary, cache = null) {
  const { outcomes } = summary;
  const lines = [
    \`Master CI runs: \${summary.total} (success=\${outcomes.success}, failure=\${outcomes.failure}, cancelled=\${outcomes.cancelled}, active/other=\${outcomes.other})\`,
    "Longest observed job executions (minutes; these are job durations, not full run wall times):",
    ...summary.slowJobs.map((job) => \`- \${job.name}: \${job.minutes.toFixed(2)}m, run \${job.runId}\`),
    "Failing jobs:",
    ...(summary.failures.length ? summary.failures.map(([name, count]) => \`- \${name}: \${count}\`) : ["- none in sampled job metadata"]),
    "Cache state (explicit Performance / Prepare log evidence only):"
  ];
  if (cache === null || Object.keys(cache).length === 0) lines.push("- unknown; supply --cache-log=PATH to inspect a retrieved job log");
  else for (const label of CACHE_LABELS) lines.push(\`- \${label}: \${cache[label] ?? "unknown"}\`);
  lines.push("Sampling is descriptive; skipped jobs are never PASS and a cache hit does not prove a performance speedup.");
  return lines.join("\n");
}

function gh(args) {
  const result = spawnSync("gh", args, { encoding: "utf8", maxBuffer: 8 * 1024 * 1024 });
  if (result.error) throw new Error("GitHub CLI unavailable: " + result.error.message);
  if (result.status !== 0) throw new Error("gh " + args[0] + " failed: " + String(result.stderr).trim());
  return JSON.parse(result.stdout);
}

export function runCiMetrics(args = process.argv.slice(2)) {
  let limit = 10;
  let cacheLogPath = null;
  for (const arg of args) {
    if (/^--limit=\d+$/.test(arg)) limit = Number(arg.split("=")[1]);
    else if (arg.startsWith("--cache-log=") && arg.length > 12) cacheLogPath = arg.slice(12);
    else throw new Error("Usage: npm run dev:ci-metrics -- [--limit=1..20] [--cache-log=PATH]");
  }
  if (!Number.isInteger(limit) || limit < 1 || limit > 20) throw new Error("Run sample must be 1..20.");
  const listing = gh(["run", "list", "--branch", "master", "--limit", String(limit),
    "--json", "databaseId,headSha,conclusion,status"]);
  const runs = listing.map((item) => ({
    ...item,
    jobs: gh(["run", "view", String(item.databaseId), "--json", "jobs"]).jobs ?? []
  }));
  const cache = cacheLogPath === null ? null : parseCacheLog(readFileSync(cacheLogPath, "utf8"));
  console.log(formatCiMetrics(summarizeCiRuns(runs), cache));
  return 0;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  try { process.exitCode = runCiMetrics(); }
  catch (error) { console.error("[ci-metrics] " + error.message); process.exitCode = 1; }
}
