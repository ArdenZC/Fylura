import fs from "node:fs";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { isValidSha } from "./ciEvidence.mjs";

const PERFORMANCE_DOMAIN_KEYS = [
  "perf_search",
  "perf_scan_schema",
  "perf_library_content",
  "perf_intelligence",
  "perf_workspace_foundation",
  "perf_preview_platform",
];

const NATIVE_PERFORMANCE_PREFIXES = [
  "src-tauri/src/platform/macos/",
  "src-tauri/src/global_index/macos/",
  "src-tauri/src/db/queries/scan.rs",
  "src-tauri/tests/macos_",
  "src-tauri/src/runtime_capabilities.rs",
  "src-tauri/src/scanner.rs",
  "src-tauri/src/file_workspace/",
  "src-tauri/tests/file_workspace_performance",
];

const WINDOWS_NATIVE_PREVIEW_HANDLER_PREFIXES = [
  "src-tauri/native/",
];

const WINDOWS_GLOBAL_INDEX_SERVICE_QUALIFICATION_PATHS = new Set([
  ".github/workflows/ci.yml",
  "src-tauri/src/lib.rs",
  "src-tauri/src/main.rs",
  "src-tauri/cargo.toml",
  "src-tauri/cargo.lock",
  "src-tauri/native-qa/global_index_probe.rs",
  "scripts/classifycichanges.mjs",
  "scripts/classifycichanges.d.mts",
  "scripts/qualifywindowsglobalindexservice.ps1",
]);

const HIGH_RISK_PREFIXES = [
  "src-tauri/src/file_ops.rs",
  "src-tauri/src/file_ops/",
  "src-tauri/src/fs_safety/",
  "src-tauri/src/db/schema.rs",
  "src-tauri/src/content.rs",
  "src-tauri/src/content/",
  "src-tauri/src/global_index/",
  "src-tauri/src/platform/macos/",
  "src-tauri/src/runtime_capabilities.rs",
  "src-tauri/src/scanner.rs",
  "src-tauri/capabilities/",
  "src-tauri/tauri.conf.json",
  "package/",
  "installer/",
  ".github/workflows/",
];

const DOC_PREFIXES = [
  "docs/",
  ".github/issue_template/",
  ".github/pull_request_template",
];

const FRONTEND_FILE_PREFIXES = ["src/", "tests/"];

const FRONTEND_FILE_NAMES = new Set([
  "index.html",
  "package.json",
  "package-lock.json",
  "vite.config.ts",
  "tsconfig.json",
  "tailwind.config.js",
  "tailwind.config.ts",
]);

const FRONTEND_INFRASTRUCTURE_PATHS = new Set([
  "scripts/w2-01-browser-gate.mjs",
  "scripts/w2-01-browser-gate.d.mts",
  "scripts/runw2-01browsergate.mjs",
  "scripts/runw2-10browsergate.mjs",
  "scripts/runw2-11browsergate.mjs",
  "scripts/runw3-10phaseabrowserharness.mjs",
  "scripts/runw3-10browsergate.mjs",
]);

function normalizePath(value) {
  return value.replaceAll("\\", "/").replace(/^\.\//, "").toLowerCase();
}

function startsWithAny(path, prefixes) {
  return prefixes.some((prefix) => path === prefix || path.startsWith(prefix));
}

function hasAnyPath(paths, predicate) {
  return paths.some((path) => predicate(normalizePath(path)));
}

function isDocumentationPath(path) {
  const normalized = normalizePath(path);
  return startsWithAny(normalized, DOC_PREFIXES)
    || normalized.endsWith(".md")
    || normalized.endsWith(".mdx")
    || /^license(?:\..*)?$/.test(normalized);
}

function isFrontendPath(path) {
  return startsWithAny(path, FRONTEND_FILE_PREFIXES)
    || FRONTEND_FILE_NAMES.has(path)
    || FRONTEND_INFRASTRUCTURE_PATHS.has(path)
    || path.endsWith(".css")
    || path.endsWith(".html");
}

function isRustPath(path) {
  return path === "rust-toolchain.toml"
    || path.startsWith("src-tauri/")
    || path === "cargo.toml"
    || path === "cargo.lock"
    || path === "src-tauri/cargo.toml"
    || path === "src-tauri/cargo.lock";
}

function isMacosSensitivePath(path) {
  return path.startsWith("src-tauri/src/platform/macos/")
    || path.startsWith("src-tauri/src/global_index/macos/")
    || path.startsWith("src-tauri/tests/macos_")
    || path.includes("/macos/");
}

function isNativePerformancePath(path) {
  return startsWithAny(path, NATIVE_PERFORMANCE_PREFIXES);
}

function isWindowsNativePreviewHandlerPath(path) {
  return startsWithAny(path, WINDOWS_NATIVE_PREVIEW_HANDLER_PREFIXES);
}

function isWindowsGlobalIndexServiceQualificationPath(path) {
  return path.startsWith("src-tauri/src/global_index/")
    || WINDOWS_GLOBAL_INDEX_SERVICE_QUALIFICATION_PATHS.has(path);
}

function isHighRiskPath(path) {
  return startsWithAny(path, HIGH_RISK_PREFIXES);
}

function isPackagePath(path) {
  return path === "package.json"
    || path === "package-lock.json"
    || path === "src-tauri/tauri.conf.json"
    || path.startsWith("src-tauri/icons/")
    || path.startsWith("installer/");
}

function isDependencyPath(path) {
  return path === "package.json"
    || path === "package-lock.json"
    || path === "src-tauri/cargo.toml"
    || path === "src-tauri/cargo.lock"
    || (path.startsWith("src-tauri/native/")
      && (path.endsWith("/cargo.toml") || path.endsWith("/cargo.lock")));
}

function isDbCorePath(path) {
  return path === "src-tauri/src/db/schema.rs"
    || path === "src-tauri/src/db/connection.rs"
    || path === "src-tauri/src/db/mod.rs"
    || path === "src-tauri/src/db/queries/mod.rs"
    || path.startsWith("src-tauri/src/db/migrations/")
    || path.startsWith("src-tauri/src/db/shared/");
}

function isSearchPath(path) {
  // "research" contains the substring "search". Strip that lexical container
  // before the legacy broad Search-name check so research-only paths do not
  // accidentally trigger the product Search performance suite.
  const searchComparablePath = path.replaceAll("research", "");
  return path.startsWith("src-tauri/src/global_index/")
    || path === "src-tauri/tests/fts_benchmark.rs"
    || searchComparablePath.includes("search")
    || path.includes("fts");
}

function isScanSchemaPath(path) {
  return path === "src-tauri/src/scanner.rs"
    || path.startsWith("src-tauri/src/db/queries/scan")
    || path === "src-tauri/tests/migrations.rs"
    || path.includes("watcher")
    || path.includes("reconcil");
}

function isLibraryContentPath(path) {
  return path.startsWith("src-tauri/src/db/queries/library/")
    || path === "src-tauri/tests/file_library_performance.rs"
    || path.startsWith("src-tauri/src/content/")
    || path === "src-tauri/src/content.rs"
    || path.startsWith("src/views/vault/")
    || path.includes("pagination")
    || path.includes("saved_view")
    || path.includes("user_tag");
}

function isIntelligencePath(path) {
  return path.startsWith("src-tauri/src/analysis")
    || path.startsWith("src-tauri/src/dedupe")
    || path.startsWith("src-tauri/src/db/queries/analysis/")
    || path.startsWith("src-tauri/src/db/queries/dedupe/")
    || path.startsWith("src-tauri/src/db/queries/organization/")
    || path.startsWith("src-tauri/src/db/queries/rule_proposals/")
    || path.startsWith("src-tauri/src/storage_analyzer");
}

function isWorkspaceFoundationPath(path) {
  return path.startsWith("src-tauri/src/file_workspace/")
    || path.startsWith("src-tauri/tests/support/file_workspace")
    || path.startsWith("src-tauri/tests/file_workspace_performance")
    || path.startsWith("src/fileworkspace/")
    || path.startsWith("src/api/fileworkspace")
    || path.startsWith("src/types/fileworkspace")
    || path.startsWith("tests/fileworkspace");
}

function isPreviewPerformancePath(path) {
  return path.startsWith("src-tauri/src/file_workspace/preview")
    || path.startsWith("src-tauri/src/file_workspace/integration/preview")
    || path.startsWith("src-tauri/src/file_workspace/integration/performance/preview")
    || path.startsWith("src-tauri/src/file_workspace/integration/performance/fixture")
    || path.startsWith("src-tauri/src/file_workspace/integration/performance/metrics")
    || path.startsWith("src-tauri/native/host-provided/")
    || path.startsWith("src-tauri/native/preview-representation/")
    || path.startsWith("src/fileworkspace/")
    || path.startsWith("src/views/filelibrary/preview/")
    || path.startsWith("tests/fileworkspace/preview")
    || path === "scripts/runw3-10phaseabrowserharness.mjs"
    || path === "scripts/runw3-10browsergate.mjs";
}

function isWorkflowPath(path) {
  return path.startsWith(".github/workflows/")
    || path === "scripts/classifycichanges.mjs"
    || path.startsWith("scripts/performance")
    || path.startsWith("scripts/runperformance")
    || path.startsWith("scripts/prepareperformance")
    || path.startsWith("scripts/checkperformance")
    || path.startsWith("src-tauri/tests/performance_fixture")
    || path.startsWith("src-tauri/tests/support/performance_fixture");
}

export function classifyCiScope({
  event = "",
  changedPaths = [],
  baseMissing = false,
  dispatchFull = false,
  prLabels = [],
} = {}) {
  const normalizedPaths = [...new Set(changedPaths.map(normalizePath))];
  const labels = new Set(prLabels.map((label) => label.trim().toLowerCase()).filter(Boolean));
  const requestedFull = event === "schedule"
    || (event === "workflow_dispatch" && dispatchFull)
    || (event === "pull_request" && labels.has("full-validation"));
  const workflowChanged = hasAnyPath(normalizedPaths, isWorkflowPath);
  const dbCoreChanged = hasAnyPath(normalizedPaths, isDbCorePath);
  const allDomains100k = requestedFull || baseMissing || workflowChanged || dbCoreChanged;

  const result = {
    docs_only: !requestedFull
      && !baseMissing
      && normalizedPaths.length > 0
      && normalizedPaths.every(isDocumentationPath),
    windows_native_preview_handler_changed: requestedFull
      || baseMissing
      || hasAnyPath(normalizedPaths, isWindowsNativePreviewHandlerPath),
    windows_global_index_service_qualification_changed: requestedFull
      || baseMissing
      || hasAnyPath(normalizedPaths, isWindowsGlobalIndexServiceQualificationPath),
    frontend_changed: requestedFull || hasAnyPath(normalizedPaths, isFrontendPath),
    rust_changed: requestedFull || hasAnyPath(normalizedPaths, isRustPath),
    macos_sensitive: requestedFull
      || hasAnyPath(normalizedPaths, (path) => isMacosSensitivePath(path) || isRustPath(path)),
    performance_sensitive: requestedFull
      || baseMissing
      || workflowChanged
      || hasAnyPath(
        normalizedPaths,
        (path) => isNativePerformancePath(path) || isWorkspaceFoundationPath(path),
      ),
    high_risk: requestedFull
      || baseMissing
      || hasAnyPath(normalizedPaths, isHighRiskPath),
    package_sensitive: requestedFull || hasAnyPath(normalizedPaths, isPackagePath),
    dependency_sensitive: requestedFull || hasAnyPath(normalizedPaths, isDependencyPath),
    release_sensitive: requestedFull
      || hasAnyPath(normalizedPaths, (path) => isRustPath(path) || isPackagePath(path)),
    all_domains_100k: allDomains100k,
    workflow_changed: workflowChanged,
    base_missing: Boolean(baseMissing),
    full_validation: requestedFull,
  };

  result.perf_search = requestedFull || allDomains100k || hasAnyPath(normalizedPaths, isSearchPath);
  result.perf_scan_schema = requestedFull || allDomains100k || hasAnyPath(normalizedPaths, isScanSchemaPath);
  result.perf_library_content = requestedFull || allDomains100k || hasAnyPath(normalizedPaths, isLibraryContentPath);
  result.perf_intelligence = requestedFull || allDomains100k || hasAnyPath(normalizedPaths, isIntelligencePath);
  result.perf_workspace_foundation = requestedFull
    || allDomains100k
    || hasAnyPath(normalizedPaths, isWorkspaceFoundationPath);
  result.perf_preview_platform = requestedFull
    || allDomains100k
    || hasAnyPath(normalizedPaths, isPreviewPerformancePath);
  result.performance_any = PERFORMANCE_DOMAIN_KEYS.some((key) => result[key]);

  if (result.docs_only) {
    for (const key of [
      "frontend_changed",
      "windows_native_preview_handler_changed",
      "windows_global_index_service_qualification_changed",
      "rust_changed",
      "macos_sensitive",
      "performance_sensitive",
      "high_risk",
      "package_sensitive",
      "dependency_sensitive",
      "release_sensitive",
      "performance_any",
      ...PERFORMANCE_DOMAIN_KEYS,
    ]) {
      result[key] = false;
    }
  }

  result.diff_head = "";
  result.diff_base = "";
  return result;
}

function readChangedPaths(base, head) {
  const diff = execFileSync("git", ["diff", "--name-status", "-M", base, head], { encoding: "utf8" });
  const changedPaths = [];
  for (const line of diff.split(/\r?\n/)) {
    if (!line) continue;
    const fields = line.split("\t");
    if (fields.length < 2) continue;
    const status = fields[0];
    const paths = fields.slice(1);
    if (status.startsWith("R") && paths.length === 2) {
      changedPaths.push(...paths);
    } else {
      changedPaths.push(paths.at(-1));
    }
  }
  return changedPaths;
}

function resolveBase(head, candidate, { requireCandidate = false } = {}) {
  if (candidate && !/^0+$/.test(candidate)) {
    if (!isValidSha(candidate)) {
      throw new Error("diff base must be a 40-character commit SHA.");
    }
    try {
      execFileSync("git", ["cat-file", "-e", `${candidate}^{commit}`], { stdio: "ignore" });
    } catch {
      throw new Error(`diff base commit is not available locally: ${candidate}`);
    }
    return { base: candidate.toLowerCase(), missing: false };
  }
  if (requireCandidate) {
    throw new Error("pull_request.base.sha is required; refusing to classify without a trusted diff base.");
  }
  const base = execFileSync("git", ["rev-list", "--max-parents=0", head], { encoding: "utf8" })
    .trim()
    .split(/\r?\n/)[0];
  return { base, missing: true };
}

function writeOutput(result) {
  const outputPath = process.env.GITHUB_OUTPUT;
  const summaryPath = process.env.GITHUB_STEP_SUMMARY;
  const entries = Object.entries(result);
  if (outputPath) {
    fs.appendFileSync(outputPath, `${entries.map(([key, value]) => `${key}=${value}`).join("\n")}\n`, "utf8");
  }
  if (summaryPath) {
    fs.appendFileSync(
      summaryPath,
      `## Change scope\n${entries.map(([key, value]) => `- ${key.replaceAll("_", " ")}: ${value}`).join("\n")}\n`,
      "utf8",
    );
  }
  console.log(JSON.stringify(result, null, 2));
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  const event = process.env.EVENT_NAME ?? "";
  const headCandidate = event === "pull_request"
    ? process.env.PR_HEAD
    : (process.env.EVENT_HEAD || execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim());
  if (!isValidSha(headCandidate)) {
    throw new Error("diff head must be a 40-character commit SHA.");
  }
  const head = headCandidate.toLowerCase();
  const requestedFull = event === "schedule"
    || (event === "workflow_dispatch" && process.env.DISPATCH_FULL?.toLowerCase() === "true")
    || (event === "pull_request" && (process.env.PR_LABELS ?? "").toLowerCase().split(",").map((label) => label.trim()).includes("full-validation"));
  const resolved = resolveBase(
    head,
    event === "pull_request" ? process.env.PR_BASE : process.env.PUSH_BASE,
    { requireCandidate: event === "pull_request" },
  );
  const result = classifyCiScope({
    event,
    changedPaths: requestedFull ? [] : readChangedPaths(resolved.base, head),
    baseMissing: resolved.missing,
    dispatchFull: process.env.DISPATCH_FULL?.toLowerCase() === "true",
    prLabels: (process.env.PR_LABELS ?? "").split(","),
  });
  result.diff_base = resolved.base;
  result.diff_head = head;
  writeOutput(result);
}
