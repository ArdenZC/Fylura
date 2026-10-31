import fs from "node:fs";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

if (process.platform !== "win32") {
  console.log(
    "[preview-handler] native Windows DLL is not built on this platform.",
  );
  process.exit(0);
}

const repositoryRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const nativeTargetDirectory = path.join(
  repositoryRoot,
  "src-tauri",
  "native",
  "target",
);
const nativeDllPath = path.join(
  nativeTargetDirectory,
  "release",
  "fylura_windows_preview_handler.dll",
);
const packageResourceDirectory = path.join(
  repositoryRoot,
  "src-tauri",
  "native",
  "packaged",
);
const packageResourcePath = path.join(
  packageResourceDirectory,
  "fylura_windows_preview_handler.dll",
);
const staticCrtRustflag = "-C target-feature=+crt-static";
const staticCrtEncodedRustflags = ["-C", "target-feature=+crt-static"].join(
  "\x1f",
);

function previewHandlerBuildEnvironment() {
  const environment = {
    ...process.env,
    CARGO_TARGET_DIR: nativeTargetDirectory,
  };
  const rustflags = String(environment.RUSTFLAGS ?? "").trim();
  if (!rustflags.includes("target-feature=+crt-static")) {
    environment.RUSTFLAGS = [rustflags, staticCrtRustflag]
      .filter(Boolean)
      .join(" ");
  }

  // Cargo gives CARGO_ENCODED_RUSTFLAGS precedence when callers provide it.
  // Preserve those caller flags and append the isolated native-DLL contract
  // rather than allowing a host environment to silently disable static CRT.
  const encodedRustflags = String(environment.CARGO_ENCODED_RUSTFLAGS ?? "");
  if (
    encodedRustflags &&
    !encodedRustflags.includes("target-feature=+crt-static")
  ) {
    environment.CARGO_ENCODED_RUSTFLAGS = `${encodedRustflags}\x1f${staticCrtEncodedRustflags}`;
  }
  return environment;
}

// The package staging file is task-owned. Remove it before building so a
// failed native build can never leave a stale DLL available to the package
// resource config.
if (fs.existsSync(packageResourcePath)) {
  fs.rmSync(packageResourcePath);
}

const result = spawnSync(
  "cargo",
  [
    "build",
    "--release",
    "--manifest-path",
    path.join(
      repositoryRoot,
      "src-tauri",
      "native",
      "windows-preview-handler",
      "Cargo.toml",
    ),
  ],
  {
    cwd: repositoryRoot,
    env: previewHandlerBuildEnvironment(),
    stdio: "inherit",
    windowsHide: true,
  },
);

if (result.error) {
  throw new Error(
    "Unable to start the Windows Preview Handler build: " +
      result.error.message,
  );
}
if (result.status !== 0) {
  process.exit(result.status ?? 1);
}

let nativeDllStats;
try {
  nativeDllStats = fs.statSync(nativeDllPath);
} catch (error) {
  throw new Error(
    "Windows Preview Handler release build did not produce the expected DLL: " +
      (error instanceof Error ? error.message : String(error)),
  );
}
if (!nativeDllStats.isFile() || nativeDllStats.size === 0) {
  throw new Error(
    `Windows Preview Handler release build produced an invalid DLL: ${nativeDllPath}`,
  );
}

fs.mkdirSync(packageResourceDirectory, { recursive: true });
fs.copyFileSync(nativeDllPath, packageResourcePath);
console.log(
  `[preview-handler] packaged production DLL: ${packageResourcePath}`,
);
