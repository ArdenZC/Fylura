import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

export function formatFailureDigest(run) {
  const name = run.workflowName || run.name || "CI";
  const lines = ["CI: " + name + " — " + (run.conclusion || run.status || "unknown")];
  if (run.url) lines.push("Run: " + run.url);
  if (run.headSha) lines.push("HEAD: " + run.headSha);
  const failures = (run.jobs || []).filter((job) => ["failure", "timed_out", "cancelled"].includes(String(job.conclusion).toLowerCase()));
  const isSummary = (job) => /^Quality \(/.test(job.name || "");
  failures.sort((a, b) => Number(isSummary(a)) - Number(isSummary(b)));
  if (failures.length === 0) lines.push("No failed jobs reported. If the run is still active, check again after completion.");
  for (const job of failures) {
    lines.push("- " + job.name + ": " + job.conclusion + (isSummary(job) && failures.length > 1 ? " (possible downstream aggregate)" : ""));
    const steps = (job.steps || []).filter((step) => ["failure", "timed_out", "cancelled"].includes(String(step.conclusion).toLowerCase()));
    steps.forEach((step) => lines.push("    step: " + step.name + " (" + step.conclusion + ")"));
  }
  lines.push("Only failed job/step metadata is summarized; inspect the original logs for the actual error. Do not interpret skipped checks as PASS.");
  return lines.join("\n");
}
export function runFailureDigest(args = process.argv.slice(2)) {
  const id = args[0];
  if (args.length !== 1 || !/^[1-9][0-9]*$/.test(id || "")) throw new Error("Usage: npm run dev:ci-failure -- RUN_ID");
  const result = spawnSync("gh", ["run", "view", id, "--json", "jobs,headSha,workflowName,conclusion,status,url"], { encoding: "utf8" });
  if (result.error) throw new Error("GitHub CLI unavailable: " + result.error.message);
  if (result.status !== 0) throw new Error("gh run view failed: " + (result.stderr || "Check gh auth status and run ID.").trim());
  console.log(formatFailureDigest(JSON.parse(result.stdout)));
  return 0;
}
if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  try { process.exitCode = runFailureDigest(); }
  catch (error) { console.error("[ci-triage] " + error.message); process.exitCode = 1; }
}
