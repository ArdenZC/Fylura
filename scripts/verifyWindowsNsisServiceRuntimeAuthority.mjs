import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";

if (process.platform !== "win32") {
  throw new Error("The NSIS service-runtime-authority smoke requires Windows.");
}

const repositoryRoot = path.resolve(import.meta.dirname, "..");
const fixturePath = path.join(
  repositoryRoot,
  "tests",
  "fixtures",
  "windows-service-runtime-authority-smoke.nsi",
);
const authorityPath = path.join(
  repositoryRoot,
  "src-tauri",
  "windows",
  "service-runtime-authority.nsh",
);
const localAppData = process.env.LOCALAPPDATA;
const candidates = [
  process.env.MAKENSIS,
  localAppData && path.join(localAppData, "tauri", "NSIS", "makensis.exe"),
  localAppData && path.join(localAppData, "tauri", "NSIS", "Bin", "makensis.exe"),
  path.join(process.env.ProgramFiles ?? "", "NSIS", "makensis.exe"),
].filter(Boolean);
const makensis = candidates.find((candidate) => fs.existsSync(candidate));

if (!makensis) {
  throw new Error(
    "makensis.exe was not found in the package toolchain candidates: " +
      candidates.join(", "),
  );
}

const systemRoot = process.env.SystemRoot ?? "C:\\Windows";
const scPath = path.join(systemRoot, "System32", "sc.exe");
if (!fs.existsSync(scPath)) {
  throw new Error("sc.exe was not found at " + scPath);
}

const tempParent = path.join(repositoryRoot, ".tmp-tests");
fs.mkdirSync(tempParent, { recursive: true });
const tempRoot = fs.mkdtempSync(
  path.join(tempParent, "service-runtime-authority-smoke-"),
);
const serviceName =
  "FyluraServiceRuntimeSmoke-" +
  process.pid +
  "-" +
  Date.now().toString(36);
const absentServiceName = serviceName + "-Absent";
const executablePath = path.join(tempRoot, "service-runtime-authority-smoke.exe");
const cleanupExecutablePath = path.join(
  tempRoot,
  "service-runtime-authority-cleanup.exe",
);
const logPath = path.join(tempRoot, "service-runtime-authority-smoke.log");

function compile(outputPath, cleanupOnly) {
  const defines = [
    "/NOCD",
    "/V4",
    "/DZC_SMOKE_OUTFILE=" + outputPath,
    "/DZC_SMOKE_SERVICE_NAME=" + serviceName,
    "/DZC_SMOKE_ABSENT_SERVICE_NAME=" + absentServiceName,
    "/DZC_SERVICE_RUNTIME_AUTHORITY_FILE=" + authorityPath,
  ];
  if (cleanupOnly) defines.push("/DZC_SMOKE_CLEANUP_ONLY");
  const result = spawnSync(makensis, [...defines, fixturePath], {
    cwd: repositoryRoot,
    encoding: "utf8",
  });
  if (result.error || result.status !== 0) {
    throw new Error(
      "Service-runtime-authority smoke compilation failed.\n" +
        (result.stdout ?? "") +
        "\n" +
        (result.stderr ?? ""),
    );
  }
  if (!fs.existsSync(outputPath) || fs.statSync(outputPath).size === 0) {
    throw new Error("Service-runtime-authority smoke produced no executable.");
  }
}

function run(executablePath) {
  return spawnSync(executablePath, [], {
    cwd: tempRoot,
    encoding: "utf8",
    windowsHide: true,
  });
}

function logContents() {
  return fs.existsSync(logPath) ? fs.readFileSync(logPath, "utf8") : "";
}

let primaryError;
let primaryLog = "";
try {
  compile(executablePath, false);
  const result = run(executablePath);
  primaryLog = logContents();
  if (result.error || result.status !== 0) {
    throw new Error(
      "Service-runtime-authority smoke failed with exit " +
        result.status +
        ".\n" +
        primaryLog +
        "\n" +
        (result.stdout ?? "") +
        "\n" +
        (result.stderr ?? ""),
    );
  }
} catch (error) {
  primaryError = error;
  if (!primaryLog) primaryLog = logContents();
}

let cleanupError;
try {
  const deleteResult = spawnSync(scPath, ["delete", serviceName], {
    cwd: tempRoot,
    encoding: "utf8",
    windowsHide: true,
  });
  if (
    deleteResult.error ||
    ![0, 1060, 1072].includes(deleteResult.status)
  ) {
    throw new Error(
      "Disposable service cleanup command failed with exit " +
        deleteResult.status +
        ".\n" +
        (deleteResult.stdout ?? "") +
        "\n" +
        (deleteResult.stderr ?? ""),
    );
  }

  compile(cleanupExecutablePath, true);
  const cleanupResult = run(cleanupExecutablePath);
  const cleanupLog = logContents();
  if (cleanupResult.error || cleanupResult.status !== 0) {
    throw new Error(
      "Disposable service cleanup verification failed with exit " +
        cleanupResult.status +
        ".\n" +
        cleanupLog +
        "\n" +
        (cleanupResult.stdout ?? "") +
        "\n" +
        (cleanupResult.stderr ?? ""),
    );
  }
} catch (error) {
  cleanupError = error;
} finally {
  try {
    fs.rmSync(tempRoot, { recursive: true, force: true });
  } catch (error) {
    cleanupError ??= error;
  }
}

if (primaryError || cleanupError) {
  const messages = [];
  if (primaryError) {
    messages.push("Primary smoke failure:\n" + primaryError.message);
  }
  if (cleanupError) {
    messages.push("Cleanup failure:\n" + cleanupError.message);
  }
  throw new Error(messages.join("\n\n"));
}

process.stdout.write(
  "Windows NSIS service-runtime-authority smoke passed with " +
    path.basename(makensis) +
    ".\n",
);
