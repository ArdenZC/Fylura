import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const repositoryRoot = path.resolve(import.meta.dirname, "..");
const previewDllName = "fylura_windows_preview_handler.dll";
const previewResourcePathForward = `native/${previewDllName}`;
const previewResourcePathBackslash = `native\\${previewDllName}`;
const flattenedPreviewResourcePath = `native${previewDllName}`;
const previewServicingPath = path.join(
  repositoryRoot,
  "src-tauri",
  "windows",
  "preview-dll-servicing.nsh",
);
const smokeFixturePath = path.join(
  repositoryRoot,
  "tests",
  "fixtures",
  "windows-preview-resource-smoke.nsi",
);

function normalizeNewlines(source) {
  return source.replace(/\r\n?/gu, "\n");
}

function previewMacroBody(source, macroName) {
  const normalized = normalizeNewlines(source);
  const start = normalized.indexOf(`!macro ${macroName}`);
  if (start < 0) {
    throw new Error(`Preview servicing macro is missing: ${macroName}`);
  }
  const end = normalized.indexOf("!macroend", start);
  if (end < 0) {
    throw new Error(`Preview servicing macro is unterminated: ${macroName}`);
  }
  return normalized.slice(start, end);
}

/**
 * Assert the two halves of the generated-resource contract:
 * the generated installer must pass the canonical backslash destination to
 * the servicing macro, and the servicing macro must emit the canonical
 * backslash destination to NSIS File. This is intentionally stronger than
 * checking only that the macro name is present.
 */
export function assertGeneratedPreviewResourcePath(
  generatedInstallerSource,
  servicingSource,
) {
  const generated = normalizeNewlines(generatedInstallerSource);
  const servicing = normalizeNewlines(servicingSource);
  const canonicalInvocation =
    `!insertmacro ZC_INSTALL_RESOURCE "${previewResourcePathBackslash}"`;
  const forwardInvocation =
    `!insertmacro ZC_INSTALL_RESOURCE "${previewResourcePathForward}"`;
  const canonicalFileInstruction =
    'File /a "/oname=${ZC_PREVIEW_DLL_RESOURCE_PATH_BACKSLASH}" "${SOURCE}"';
  const expandedCanonicalFileInstruction =
    `File /a "/oname=${previewResourcePathBackslash}" ` + '"${SOURCE}"';

  if (!generated.includes(canonicalInvocation)) {
    throw new Error(
      `Generated installer does not pass the canonical Preview resource path: ${canonicalInvocation}`,
    );
  }
  if (generated.includes(forwardInvocation)) {
    throw new Error(
      `Generated installer still passes a forward-slash Preview resource path: ${forwardInvocation}`,
    );
  }

  const installPreview = previewMacroBody(servicing, "ZC_INSTALL_PREVIEW_RESOURCE");
  if (!installPreview.includes(canonicalFileInstruction)) {
    throw new Error(
      `Preview servicing does not emit the canonical NSIS File destination: ${canonicalFileInstruction}`,
    );
  }
  if (installPreview.includes('File /a "/oname=${DESTINATION}" "${SOURCE}"')) {
    throw new Error(
      "Preview servicing still forwards a macro destination into NSIS File.",
    );
  }

  const installResource = previewMacroBody(servicing, "ZC_INSTALL_RESOURCE");
  if (!installResource.includes("ZC_PREVIEW_DLL_RESOURCE_PATH_FORWARD")
    || !installResource.includes("ZC_PREVIEW_DLL_RESOURCE_PATH_BACKSLASH")
    || !installResource.includes("ZC_INSTALL_PREVIEW_RESOURCE")) {
    throw new Error(
      "Preview resource identity matching no longer covers both path representations.",
    );
  }

  return {
    canonicalInvocation,
    canonicalFileInstruction: expandedCanonicalFileInstruction,
  };
}

function targetDirectory() {
  const configured = process.env.CARGO_TARGET_DIR;
  if (configured) {
    return path.isAbsolute(configured)
      ? configured
      : path.resolve(repositoryRoot, configured);
  }
  return path.join(repositoryRoot, "src-tauri", "target");
}

export function resolveGeneratedNsisInstallerPath() {
  const candidates = [
    path.join(targetDirectory(), "release", "nsis", "x64", "installer.nsi"),
    path.join(repositoryRoot, "src-tauri", "target", "release", "nsis", "x64", "installer.nsi"),
    path.join(repositoryRoot, "target", "release", "nsis", "x64", "installer.nsi"),
  ];
  return candidates.find((candidate) => fs.existsSync(candidate)) ?? candidates[0];
}

export function assertGeneratedPreviewResourcePathFile(
  generatedInstallerPath = resolveGeneratedNsisInstallerPath(),
) {
  if (!fs.existsSync(generatedInstallerPath)) {
    throw new Error(`Generated NSIS installer script is missing: ${generatedInstallerPath}`);
  }
  return assertGeneratedPreviewResourcePath(
    fs.readFileSync(generatedInstallerPath, "utf8"),
    fs.readFileSync(previewServicingPath, "utf8"),
  );
}

function findMakeNsis() {
  const localAppData = process.env.LOCALAPPDATA;
  const programFilesX86 = process.env["ProgramFiles(x86)"];
  const programFilesW6432 = process.env.ProgramW6432;
  const pathEntries = (process.env.Path ?? process.env.PATH ?? "")
    .split(path.delimiter)
    .filter(Boolean);
  const candidates = [
    process.env.MAKENSIS,
    localAppData && path.join(localAppData, "tauri", "NSIS", "makensis.exe"),
    localAppData && path.join(localAppData, "tauri", "NSIS", "Bin", "makensis.exe"),
    path.join(process.env.ProgramFiles ?? "", "NSIS", "makensis.exe"),
    programFilesW6432 && path.join(programFilesW6432, "NSIS", "makensis.exe"),
    programFilesX86 && path.join(programFilesX86, "NSIS", "makensis.exe"),
    ...pathEntries.map((entry) => path.join(entry, "makensis.exe")),
  ].filter(Boolean);
  return candidates.find((candidate) => fs.existsSync(candidate));
}

function commandFailure(label, result) {
  return new Error(
    `${label} failed with exit ${result.status}.\n` +
      `${result.stdout ?? ""}\n${result.stderr ?? ""}`.slice(-12000),
    { cause: result.error },
  );
}

function requireFile(pathValue, label) {
  if (!fs.existsSync(pathValue) || !fs.statSync(pathValue).isFile()) {
    throw new Error(`${label} is missing: ${pathValue}`);
  }
}

function inspectOutputRoot(root, sourceBytes, label) {
  const nativeDirectory = path.join(root, "native");
  const canonical = path.join(nativeDirectory, previewDllName);
  const flattened = path.join(root, flattenedPreviewResourcePath);
  if (!fs.existsSync(nativeDirectory) || !fs.statSync(nativeDirectory).isDirectory()) {
    throw new Error(`${label} did not retain the native parent directory: ${nativeDirectory}`);
  }
  requireFile(canonical, `${label} canonical Preview DLL`);
  if (fs.existsSync(flattened)) {
    throw new Error(`${label} created the flattened Preview DLL path: ${flattened}`);
  }
  const outputBytes = fs.readFileSync(canonical);
  if (!outputBytes.equals(sourceBytes)) {
    throw new Error(
      `${label} canonical Preview DLL bytes differ from the packaged source ` +
        `(source=${crypto.createHash("sha256").update(sourceBytes).digest("hex")}, ` +
        `output=${crypto.createHash("sha256").update(outputBytes).digest("hex")}).`,
    );
  }
  return canonical;
}

export function runWindowsPreviewResourceSmoke({ sourcePath } = {}) {
  if (process.platform !== "win32") {
    throw new Error("The Windows Preview resource smoke requires Windows.");
  }
  const makensis = findMakeNsis();
  if (!makensis) {
    throw new Error("makensis.exe was not found in the package toolchain candidates.");
  }
  const resolvedSourcePath = path.resolve(
    repositoryRoot,
    sourcePath ?? path.join("src-tauri", "native", "packaged", previewDllName),
  );
  requireFile(resolvedSourcePath, "Packaged Preview Handler DLL");
  const sourceBytes = fs.readFileSync(resolvedSourcePath);

  const tempParent = path.join(repositoryRoot, ".tmp-tests");
  fs.mkdirSync(tempParent, { recursive: true });
  const tempRoot = fs.mkdtempSync(
    path.join(tempParent, "windows-preview-resource-smoke-"),
  );
  const executablePath = path.join(tempRoot, "windows-preview-resource-smoke.exe");

  try {
    const compile = spawnSync(
      makensis,
      [
        "/NOCD",
        "/V4",
        `/DZC_SMOKE_OUTFILE=${executablePath}`,
        `/DZC_SMOKE_SOURCE=${resolvedSourcePath}`,
        `/DZC_PREVIEW_DLL_SERVICING_FILE=${previewServicingPath}`,
        smokeFixturePath,
      ],
      { cwd: repositoryRoot, encoding: "utf8", windowsHide: true },
    );
    if (compile.error || compile.status !== 0) {
      throw commandFailure("Preview resource smoke compilation", compile);
    }
    requireFile(executablePath, "Preview resource smoke executable");

    const run = spawnSync(executablePath, ["/S", "/NCRC"], {
      // This is a silent, zero-page installer fixture. Pass the explicit
      // silent switch so the smoke always executes its section on Windows.
      // /NCRC only avoids an unrelated self-check; it does not alter the
      // resource extraction or servicing paths under test.
      cwd: tempRoot,
      encoding: "utf8",
      windowsHide: true,
    });
    if (run.error || run.status !== 0) {
      throw commandFailure("Preview resource smoke execution", run);
    }

    const freshRoot = path.join(tempRoot, "fresh-root");
    const mappedRoot = path.join(tempRoot, "mapped-root");
    const freshCanonical = inspectOutputRoot(freshRoot, sourceBytes, "fresh resource path");
    const mappedCanonical = inspectOutputRoot(mappedRoot, sourceBytes, "mapped resource path");
    const retirementDirectory = path.join(tempRoot, ".fylura-retired");
    if (fs.existsSync(retirementDirectory)) {
      throw new Error(
        `mapped resource path left the retirement directory after finalization: ${retirementDirectory}`,
      );
    }

    process.stdout.write(
      "Windows Preview resource smoke passed: fresh canonical extraction, " +
        "mapped retirement/replacement, and canonical byte identity.\n" +
        `fresh=${freshCanonical}\nmapped=${mappedCanonical}\n`,
    );
    return { freshCanonical, mappedCanonical };
  } finally {
    fs.rmSync(tempRoot, { recursive: true, force: true });
  }
}

function optionValue(argv, name) {
  const prefix = `${name}=`;
  const inline = argv.find((argument) => argument.startsWith(prefix));
  if (inline) return inline.slice(prefix.length);
  const index = argv.indexOf(name);
  return index >= 0 ? argv[index + 1] : undefined;
}

if (process.argv[1]
  && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  if (process.platform !== "win32") {
    throw new Error("The Windows Preview resource verifier requires Windows.");
  }
  const installerPath = optionValue(process.argv.slice(2), "--installer");
  const sourcePath = optionValue(process.argv.slice(2), "--source");
  if (installerPath) {
    assertGeneratedPreviewResourcePathFile(path.resolve(installerPath));
  } else {
    const resolvedInstaller = resolveGeneratedNsisInstallerPath();
    if (fs.existsSync(resolvedInstaller)) {
      assertGeneratedPreviewResourcePathFile(resolvedInstaller);
    } else {
      process.stdout.write(
        `Generated NSIS Preview resource assertion deferred; script not found: ${resolvedInstaller}\n`,
      );
    }
  }
  runWindowsPreviewResourceSmoke({ sourcePath });
}
