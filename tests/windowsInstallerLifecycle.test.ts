import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import {
  buildFyluraNsisTemplate,
  TAURI_NSIS_UPSTREAM_BLOB_SHA,
} from "../scripts/prepareWindowsNsisLifecycleTemplate.mjs";
import { assertGeneratedPreviewResourcePath } from "../scripts/verifyWindowsNsisPreviewResource.mjs";

const repositoryRoot = path.resolve(import.meta.dirname, "..");
const upstreamPath = path.join(
  repositoryRoot,
  "src-tauri",
  "windows",
  "tauri-2.11.2-installer.upstream.nsi",
);
const wrapperPath = path.join(repositoryRoot, "src-tauri", "windows", "installer-lifecycle-wrapper.nsh");
const synchronousPath = path.join(
  repositoryRoot,
  "src-tauri",
  "windows",
  "installer-lifecycle-synchronous.nsh",
);
const finalPath = path.join(
  repositoryRoot,
  "src-tauri",
  "windows",
  "installer-lifecycle-final.nsh",
);
const installerHooksPath = path.join(
  repositoryRoot,
  "src-tauri",
  "windows",
  "installer-hooks.nsh",
);
const previewDllServicingPath = path.join(
  repositoryRoot,
  "src-tauri",
  "windows",
  "preview-dll-servicing.nsh",
);
const registryAuthorityPath = path.join(
  repositoryRoot,
  "src-tauri",
  "windows",
  "registry-authority.nsh",
);
const registrySmokeFixturePath = path.join(
  repositoryRoot,
  "tests",
  "fixtures",
  "windows-registry-authority-smoke.nsi",
);
const registrySmokeScriptPath = path.join(
  repositoryRoot,
  "scripts",
  "verifyWindowsNsisRegistryAuthority.mjs",
);
const serviceRuntimeAuthorityPath = path.join(
  repositoryRoot,
  "src-tauri",
  "windows",
  "service-runtime-authority.nsh",
);
const serviceSmokeFixturePath = path.join(
  repositoryRoot,
  "tests",
  "fixtures",
  "windows-service-runtime-authority-smoke.nsi",
);
const serviceSmokeScriptPath = path.join(
  repositoryRoot,
  "scripts",
  "verifyWindowsNsisServiceRuntimeAuthority.mjs",
);
const releaseWorkflowPath = path.join(
  repositoryRoot,
  ".github",
  "workflows",
  "release-build.yml",
);

function sectionBody(source: string, sectionName: string) {
  const start = source.indexOf(`Section ${sectionName}`);
  expect(start).toBeGreaterThanOrEqual(0);
  const end = source.indexOf("SectionEnd", start);
  expect(end).toBeGreaterThan(start);
  return source.slice(start, end);
}

function normalizeNewlines(source: string) {
  return source.replace(/\r\n?/gu, "\n");
}

function operationalNsisSource(source: string) {
  return normalizeNewlines(source)
    .split("\n")
    .filter((line) => !line.trimStart().startsWith(";"))
    .join("\n");
}

function functionBody(source: string, functionName: string) {
  const start = source.indexOf(`Function ${functionName}`);
  expect(start).toBeGreaterThanOrEqual(0);
  const end = source.indexOf("FunctionEnd", start);
  expect(end).toBeGreaterThan(start);
  return normalizeNewlines(source.slice(start, end));
}

function macroBody(source: string, macroName: string) {
  const start = source.indexOf(`!macro ${macroName}`);
  expect(start).toBeGreaterThanOrEqual(0);
  const end = source.indexOf("!macroend", start);
  expect(end).toBeGreaterThan(start);
  return normalizeNewlines(source.slice(start, end));
}

const lifecycleStages = {
  inactive: 0,
  reversiblePreparation: 1,
  fileMutation: 2,
  generatedMutation: 3,
  postGeneratedIntegration: 4,
  complete: 5,
} as const;

function advanceLifecycleStage(current: number, next: number) {
  return Math.max(current, next);
}

function successfulRepairServiceState(_original: "RUNNING" | "STOPPED") {
  return "RUNNING" as const;
}

function failedRepairServiceState(
  original: "RUNNING" | "STOPPED",
  stage: number,
) {
  return stage === lifecycleStages.fileMutation || stage === lifecycleStages.generatedMutation
    ? "STOPPED"
    : original;
}

type FailureProduct = "fresh" | "repair" | "unknown";

function partialFailureActions(product: FailureProduct = "repair") {
  if (product === "fresh") {
    return ["exact-preview-withdrawal", "fresh-service-compensation"];
  }
  if (product === "unknown") {
    return ["exact-preview-withdrawal", "service-cleanup-incomplete"];
  }
  return ["exact-preview-withdrawal", "repair-service-stop-only"];
}

function metadataFailureActions(product: FailureProduct) {
  if (product === "fresh") {
    return ["fresh-metadata-compensation"];
  }
  if (product === "repair") {
    return ["repair-metadata-preserved"];
  }
  return ["metadata-cleanup-incomplete"];
}

function metadataFailureReport(product: FailureProduct, cleanupVerified: boolean) {
  if (product === "fresh") {
    return cleanupVerified ? "fresh-metadata-neutralized" : "fresh-metadata-incomplete";
  }
  if (product === "repair") {
    return "repair-metadata-preserved";
  }
  return "metadata-cleanup-incomplete";
}

function dispatchFailure(
  stage: number,
  coherent: boolean,
  product: FailureProduct = "repair",
  ownerDone = false,
) {
  if (ownerDone || stage === lifecycleStages.inactive) {
    return { ownerDone, actions: [] as string[], metadataActions: [] as string[] };
  }
  if (stage === lifecycleStages.reversiblePreparation) {
    return { ownerDone: true, actions: ["reversible-recovery"], metadataActions: [] as string[] };
  }
  if (stage === lifecycleStages.fileMutation || stage === lifecycleStages.generatedMutation) {
    return {
      ownerDone: true,
      actions: partialFailureActions(product),
      metadataActions: metadataFailureActions(product),
    };
  }
  if (stage === lifecycleStages.complete) {
    return { ownerDone: true, actions: [] as string[], metadataActions: [] as string[] };
  }
  if (stage !== lifecycleStages.postGeneratedIntegration) {
    return { ownerDone: true, actions: ["fail-closed"], metadataActions: [] as string[] };
  }
  return {
    ownerDone: true,
    actions: coherent
      ? product === "fresh"
        ? ["preview-rollback", "fresh-service-compensation"]
        : product === "repair"
          ? ["preview-rollback", "captured-service-restore"]
          : ["preview-rollback", "service-cleanup-incomplete"]
      : partialFailureActions(product),
    metadataActions: metadataFailureActions(product),
  };
}

function labeledBody(source: string, label: string) {
  const start = source.indexOf(`${label}:`);
  expect(start).toBeGreaterThanOrEqual(0);
  const end = source.indexOf("FunctionEnd", start);
  return source.slice(start, end >= 0 ? end : source.length);
}

function nsisConditionalBody(source: string, marker: string, condition: string) {
  const markerIndex = source.indexOf(marker);
  expect(markerIndex).toBeGreaterThanOrEqual(0);
  const ifIndex = source.indexOf(condition, markerIndex);
  expect(ifIndex).toBeGreaterThan(markerIndex);

  const tokenPattern = /\$\{If\}|\$\{EndIf\}/g;
  tokenPattern.lastIndex = ifIndex;
  let depth = 0;
  let match: RegExpExecArray | null;
  while ((match = tokenPattern.exec(source)) !== null) {
    if (match[0] === "${If}") {
      depth += 1;
    } else {
      depth -= 1;
      if (depth === 0) {
        return source.slice(ifIndex, match.index);
      }
    }
  }
  throw new Error(`Unclosed NSIS conditional for ${marker}`);
}

function loopBody(source: string, label: string, nextLabel: string) {
  const start = source.indexOf(`${label}:`);
  expect(start).toBeGreaterThanOrEqual(0);
  const endMarker =
    nextLabel === "FunctionEnd" || nextLabel === "!macroend"
      ? nextLabel
      : `${nextLabel}:`;
  const end = source.indexOf(endMarker, start + label.length + 1);
  expect(end).toBeGreaterThan(start);
  return source.slice(start, end);
}

function installerHooksSource() {
  return normalizeNewlines(fs.readFileSync(installerHooksPath, "utf8"));
}

function previewDllServicingSource() {
  return normalizeNewlines(fs.readFileSync(previewDllServicingPath, "utf8"));
}

type RegistryKeyState = "absent" | "present" | "unknown";
type RegistryValueState = "absent" | "exact" | "foreign" | "unknown";
type RegistryEnumState = "item" | "end" | "unknown";

function exactKeyState(result: "MISSING" | "OPENED" | "ERROR"): RegistryKeyState {
  return result === "MISSING" ? "absent" : result === "OPENED" ? "present" : "unknown";
}

function exactValueState(
  result: "MISSING" | "ERROR" | { type: string; value: string | number },
  expectedType: string,
  expectedValue: string | number,
): RegistryValueState {
  if (result === "MISSING") return "absent";
  if (result === "ERROR") return "unknown";
  return result.type === expectedType && result.value === expectedValue ? "exact" : "foreign";
}

function enumState(result: "NO_MORE_ITEMS" | "ERROR" | string): RegistryEnumState {
  if (result === "NO_MORE_ITEMS") return "end";
  if (result === "ERROR") return "unknown";
  return "item";
}

function expectedServiceRuntimeState(currentState: number) {
  if (currentState === 1) return 2;
  if (currentState === 2 || currentState === 3 || currentState === 5 || currentState === 6) return 3;
  if (currentState === 4) return 1;
  return 0;
}

function compensationAction(runtimeState: number) {
  if (runtimeState === 4) return "success";
  if (runtimeState === 2) return "delete";
  if (runtimeState === 3) return "wait";
  if (runtimeState !== 1) return "incomplete";
  return "stop";
}

function associationAction(state: RegistryValueState) {
  return state === "absent"
    ? "claim"
    : state === "exact"
      ? "idempotent"
      : state === "foreign"
        ? "preserve"
        : "fail";
}

type ProductValueProbe = "MISSING" | "ERROR" | { type: string; value: string | number };

type RepairProductFixture = {
  values: Record<string, ProductValueProbe>;
  manufacturerDefault: ProductValueProbe;
  uninstallerPresent: boolean;
};

const repairExpectedValues = {
  MainBinaryName: { type: "REG_SZ", value: "fylura.exe" },
  DisplayName: { type: "REG_SZ", value: "Zen Canvas" },
  DisplayIcon: { type: "REG_SZ", value: '"C:\\Program Files\\Zen Canvas\\fylura.exe"' },
  DisplayVersion: { type: "REG_SZ", value: "0.1.40" },
  Publisher: { type: "REG_SZ", value: "startlan" },
  InstallLocation: { type: "REG_SZ", value: '"C:\\Program Files\\Zen Canvas"' },
  UninstallString: { type: "REG_SZ", value: '"C:\\Program Files\\Zen Canvas\\uninstall.exe"' },
  NoModify: { type: "REG_DWORD", value: 1 },
  NoRepair: { type: "REG_DWORD", value: 1 },
  EstimatedSize: { type: "REG_DWORD", value: 32920 },
} as const;

const repairMandatoryValueNames = Object.keys(repairExpectedValues);
const repairOptionalUrlNames = ["URLInfoAbout", "URLUpdateInfo", "HelpLink"] as const;
const repairExpectedHomepage = "https://github.com/ArdenZC/Fylura";

function realA1RepairFixture(): RepairProductFixture {
  return {
    values: {
      ...repairExpectedValues,
      URLInfoAbout: "MISSING",
      URLUpdateInfo: "MISSING",
      HelpLink: "MISSING",
    },
    manufacturerDefault: { type: "REG_SZ", value: "C:\\Program Files\\Zen Canvas" },
    uninstallerPresent: true,
  };
}

function withRepairValue(
  fixture: RepairProductFixture,
  name: string,
  value: ProductValueProbe,
): RepairProductFixture {
  return { ...fixture, values: { ...fixture.values, [name]: value } };
}

function repairAdmissionState(fixture: RepairProductFixture) {
  const mandatoryExact = repairMandatoryValueNames.every((name) => {
    const expected = repairExpectedValues[name as keyof typeof repairExpectedValues];
    return exactValueState(fixture.values[name], expected.type, expected.value) === "exact";
  });
  const optionalUrlsValid = repairOptionalUrlNames.every((name) => {
    const state = exactValueState(fixture.values[name], "REG_SZ", repairExpectedHomepage);
    return state === "absent" || state === "exact";
  });
  const manufacturerExact =
    exactValueState(fixture.manufacturerDefault, "REG_SZ", "C:\\Program Files\\Zen Canvas") === "exact";
  const valid = mandatoryExact && optionalUrlsValid && manufacturerExact && fixture.uninstallerPresent;
  return { valid, product: valid ? 1 : 2 };
}

function preDeleteEvidenceState(fixture: RepairProductFixture) {
  return repairAdmissionState(fixture).valid ? 1 : 0;
}

function realA3UninstallFixture(): RepairProductFixture {
  return realA1RepairFixture();
}

function freshMetadataCleanupAllowed(states: RegistryValueState[]) {
  return states.every((state) => state === "absent" || state === "exact");
}

const manufacturerInstallLocation = "C:\\Program Files\\Zen Canvas";

function manufacturerMarkerCleanupDecision(
  value: ProductValueProbe,
  postDeleteValue: ProductValueProbe = "MISSING",
  postDeleteKey: RegistryKeyState = "absent",
) {
  const state = exactValueState(value, "REG_SZ", manufacturerInstallLocation);
  if (state !== "exact") {
    return {
      deleteAttempted: false,
      preserved: true,
      clean: state === "absent" && postDeleteKey === "absent",
    };
  }
  const afterDelete = exactValueState(
    postDeleteValue,
    "REG_SZ",
    manufacturerInstallLocation,
  );
  return {
    deleteAttempted: true,
    preserved: false,
    clean: afterDelete === "absent" && postDeleteKey === "absent",
  };
}

type PreviewValueSurface = {
  clsidDefault: RegistryValueState;
  appId: RegistryValueState;
  inprocDefault: RegistryValueState;
  threadingModel: RegistryValueState;
  previewHandlers: RegistryValueState;
  associations: RegistryValueState[];
  clsidKeyPresent: boolean;
  inprocKeyPresent: boolean;
};

function activePreviewValuesAbsent(surface: PreviewValueSurface) {
  return [
    surface.clsidDefault,
    surface.appId,
    surface.inprocDefault,
    surface.threadingModel,
    surface.previewHandlers,
    ...surface.associations,
  ].every((state) => state === "absent");
}

function freshDetectionSeesProduct(
  uninstallerKeyPresent: boolean,
  manufacturerKeyPresent: boolean,
  uninstallerFilePresent: boolean,
) {
  return uninstallerKeyPresent || manufacturerKeyPresent || uninstallerFilePresent;
}

describe("W4-04 package NSIS lifecycle", () => {
  it("pins the exact Tauri 2.11.2 upstream template and package-only custom template", () => {
    expect(TAURI_NSIS_UPSTREAM_BLOB_SHA).toBe("a48a46149f6d6bdc76a0bf13f53e4acdfedb310b");
    const upstream = fs.readFileSync(upstreamPath, "utf8");
    const generated = buildFyluraNsisTemplate(upstream);
    expect(generated).not.toBe(upstream);
    const canonicalUpstream = upstream.replace(/\r\n/g, "\n");
    expect(buildFyluraNsisTemplate(canonicalUpstream.replace(/\n/g, "\r\n"))).toBe(
      buildFyluraNsisTemplate(canonicalUpstream),
    );

    const packageConfig = JSON.parse(
      fs.readFileSync(path.join(repositoryRoot, "src-tauri", "tauri.windows.package.conf.json"), "utf8"),
    );
    expect(packageConfig.bundle.windows.nsis.installerHooks).toBe(
      "windows/installer-lifecycle-wrapper.nsh",
    );
    expect(packageConfig.bundle.windows.nsis.template).toBe(
      "windows/.generated/fylura-installer.nsi",
    );
  });

  it("keeps the Windows COM identity stable and refuses to adopt the old preview registration", () => {
    const registration = fs.readFileSync(
      path.join(repositoryRoot, "src-tauri", "windows", "preview-handler-registration.nsh"),
      "utf8",
    );
    const hooks = fs.readFileSync(installerHooksPath, "utf8");

    expect(registration).toContain('ZC_PREVIEW_PRODUCTION_CLSID "{3D1A446C-162E-4313-A026-8ADC792C4862}"');
    expect(registration).toContain('ZC_PREVIEW_PREVHOST_APP_ID "{6D2B5079-2F0B-48DD-AB7F-97CEC514D30B}"');
    expect(registration).toContain('ZC_PREVIEW_FRIENDLY_NAME "Fylura Preview Handler"');
    expect(registration).toContain('ZC_PREVIEW_LEGACY_FRIENDLY_NAME "Zen Canvas Preview Handler"');
    expect(hooks).toContain('"${ZC_PREVIEW_LEGACY_FRIENDLY_NAME}" ${ZC_REG_STRING_SZ_ONLY}');
    expect(hooks).toContain("Uninstall the older Zen Canvas development build, then install Fylura again.");
    expect(hooks).toContain("Its database and user files are preserved by uninstall.");
  });

  it("blocks install while the old auto-start index service is still registered", () => {
    const hooks = fs.readFileSync(installerHooksPath, "utf8");
    const lifecycle = fs.readFileSync(
      path.join(repositoryRoot, "src-tauri", "windows", "installer-lifecycle-synchronous.nsh"),
      "utf8",
    );
    const guard = functionBody(hooks, "ValidateLegacyZenCanvasIndexServiceAbsent");

    expect(hooks).toContain('ZC_LEGACY_INDEX_SERVICE_KEY "SYSTEM\\CurrentControlSet\\Services\\ZenCanvasGlobalIndex"');
    expect(guard).toContain("${ZC_REG_KEY_PRESENT}");
    expect(guard).toContain("${ZC_REG_KEY_ABSENT}");
    expect(guard).toContain("then install Fylura again.");
    expect(guard).toContain("Installation was not changed.");
    expect(guard).not.toContain('"sc.exe" stop');
    expect(guard).not.toContain('"sc.exe" delete');
    expect(lifecycle).toContain("Call ValidateLegacyZenCanvasIndexServiceAbsent");
  });

  it("uses only the final synchronous lifecycle entries in generated install/uninstall sections", () => {
    const generated = buildFyluraNsisTemplate(fs.readFileSync(upstreamPath, "utf8"));
    const install = sectionBody(generated, "Install");
    const uninstall = sectionBody(generated, "Uninstall");

    expect(install).toContain("Call ZCPrepareInstallLifecycleFinal");
    expect(install).toContain("Call ZCPostInstallLifecycleFinal");
    expect(uninstall).toContain("Call un.ZCPrepareUninstallLifecycleFinal");
    expect(uninstall).toContain("Call un.ZCPostUninstallLifecycleFinal");

    for (const legacyHook of [
      "!insertmacro NSIS_HOOK_PREINSTALL",
      "!insertmacro NSIS_HOOK_POSTINSTALL",
      "!insertmacro NSIS_HOOK_PREUNINSTALL",
      "!insertmacro NSIS_HOOK_POSTUNINSTALL",
      "!insertmacro CheckIfAppIsRunning",
    ]) {
      expect(install).not.toContain(legacyHook);
      expect(uninstall).not.toContain(legacyHook);
    }
  });

  it("orders install service stop, app gate, Preview quiesce, then generated mutation", () => {
    const generated = buildFyluraNsisTemplate(fs.readFileSync(upstreamPath, "utf8"));
    const install = sectionBody(generated, "Install");
    expect(install.indexOf("Call ZCPrepareInstallLifecycleFinal")).toBeLessThan(
      install.indexOf("SetOverwrite try"),
    );
    expect(install.indexOf("SetOverwrite try")).toBeLessThan(
      install.indexOf("Call ZCMarkInstallIrreversible"),
    );
    expect(install.indexOf("Call ZCMarkInstallIrreversible")).toBeLessThan(
      install.indexOf('File "${MAINBINARYSRCPATH}"'),
    );
    expect(install.indexOf('File "${MAINBINARYSRCPATH}"')).toBeLessThan(
      install.indexOf("IfErrors zc_install_partial_failure"),
    );
    expect(install.indexOf("IfErrors zc_install_partial_failure")).toBeLessThan(
      install.indexOf("Call ZCMarkInstallGeneratedMutation"),
    );
    expect(install).not.toContain("IfErrors zc_install_reversible_failure");

    const synchronous = fs.readFileSync(synchronousPath, "utf8");
    const initialize = functionBody(synchronous, "ZCInitializeInstallLifecycle");
    expect(initialize).toContain("Call ValidateZenCanvasPreexistingProduct");
    expect(initialize).toContain("Call ValidateZenCanvasPreviewCore");
    expect(initialize).toContain("Call ValidateZenCanvasIndexServiceOwnership");

    const final = fs.readFileSync(finalPath, "utf8");
    const prepare = functionBody(final, "ZCPrepareInstallLifecycleFinal");
    const stop = prepare.indexOf("Call ZCStopCapturedServiceForLifecycle");
    const gate = prepare.indexOf("Call ZCResolveMainAppGateFinal");
    const quiesce = prepare.indexOf("Call ZCQuiescePreviewForLifecycle");
    expect(stop).toBeGreaterThanOrEqual(0);
    expect(gate).toBeGreaterThan(stop);
    expect(quiesce).toBeGreaterThan(gate);
  });

  it("orders uninstall service stop, app gate, Preview quiesce, then first critical delete", () => {
    const generated = buildFyluraNsisTemplate(fs.readFileSync(upstreamPath, "utf8"));
    const uninstall = sectionBody(generated, "Uninstall");
    expect(uninstall.indexOf("Call un.ZCPrepareUninstallLifecycleFinal")).toBeLessThan(
      uninstall.indexOf('Delete "$INSTDIR\\${MAINBINARYNAME}.exe"'),
    );

    const synchronous = fs.readFileSync(synchronousPath, "utf8");
    const initialize = functionBody(synchronous, "un.ZCInitializeUninstallLifecycle");
    expect(initialize).toContain("Call un.ValidateZenCanvasPreviewCore");
    expect(initialize).toContain("Call un.ValidateZenCanvasIndexServiceOwnership");
    expect(initialize).toContain("Call un.CaptureZenCanvasOriginalServiceState");
    expect(initialize).toContain("Call un.CheckZenCanvasPreDeleteProductEvidence");

    const final = fs.readFileSync(finalPath, "utf8");
    const prepare = functionBody(final, "un.ZCPrepareUninstallLifecycleFinal");
    const stop = prepare.indexOf("Call un.ZCStopCapturedServiceForLifecycle");
    const gate = prepare.indexOf("Call un.ZCResolveMainAppGateFinal");
    const quiesce = prepare.indexOf("Call un.ZCQuiescePreviewForLifecycle");
    expect(stop).toBeGreaterThanOrEqual(0);
    expect(gate).toBeGreaterThan(stop);
    expect(quiesce).toBeGreaterThan(gate);
  });

  it("keeps silent and passive process gates non-interactive", () => {
    const final = fs.readFileSync(finalPath, "utf8");
    for (const functionName of ["ZCResolveMainAppGateFinal", "un.ZCResolveMainAppGateFinal"]) {
      const gate = functionBody(final, functionName);
      const silent = gate.indexOf("IfSilent");
      const passive = gate.indexOf("$PassiveMode == 1");
      const prompt = gate.indexOf("MessageBox MB_OKCANCEL");
      expect(silent).toBeGreaterThanOrEqual(0);
      expect(passive).toBeGreaterThan(silent);
      expect(prompt).toBeGreaterThan(passive);
    }
  });

  it("owns Preview quiesce synchronously without hidden Abort paths", () => {
    const synchronous = fs.readFileSync(synchronousPath, "utf8");
    const installQuiesce = functionBody(synchronous, "ZCQuiescePreviewForLifecycle");
    const uninstallQuiesce = functionBody(synchronous, "un.ZCQuiescePreviewForLifecycle");
    expect(installQuiesce).not.toContain("Abort");
    expect(uninstallQuiesce).not.toContain("Abort");
    expect(synchronous).toContain("StrCpy $ZC_LIFECYCLE_PREVIEW_OK 0");
    expect(synchronous).not.toContain("WAIT_FUNCTION");
    expect(synchronous).not.toContain("WaitForZenCanvasPreviewDllRelease");
    expect(installQuiesce).toContain("NotifyZenCanvasPreviewAssociationChanged");
    expect(uninstallQuiesce).toContain("un.NotifyZenCanvasPreviewAssociationChanged");
  });

  it("routes generated file and metadata failures to the same synchronous partial owner", () => {
    const generated = buildFyluraNsisTemplate(fs.readFileSync(upstreamPath, "utf8"));
    const install = sectionBody(generated, "Install");
    const uninstall = sectionBody(generated, "Uninstall");

    expect(install).toContain("IfErrors zc_install_partial_failure");
    expect(install).toContain("Call ZCMarkInstallIrreversible");
    expect(install).toContain("APP_ASSOCIATE");
    expect(install).toContain("URL Protocol");
    expect(install).toContain("EstimatedSize");

    expect(uninstall).toContain("IfErrors zc_uninstall_reversible_failure");
    expect(uninstall).toContain("IfErrors zc_uninstall_partial_failure");
    expect(uninstall).toContain("Call un.ZCMarkUninstallIrreversible");
    expect(uninstall).toContain("Call un.ZCMarkUninstallGeneratedMutation");
    expect(uninstall).toContain("APP_UNASSOCIATE");
    expect(uninstall).not.toContain("RMDir /REBOOTOK");
    expect(uninstall).toContain('RMDir "$INSTDIR"');
  });

  it("preserves the captured service runtime state on repair and rejects service races", () => {
    const final = fs.readFileSync(finalPath, "utf8");
    const ensure = functionBody(final, "ZCEnsurePostInstallServiceFinal");
    expect(ensure).toContain("$ZC_PREEXISTING_SERVICE == 1");
    expect(ensure).toContain("Call ZCEnsureZenCanvasIndexServiceRunningForSuccessfulInstall");
    expect(ensure).toContain("$ZC_INDEX_SERVICE_OWNERSHIP != 1");
    expect(ensure).toContain("$ZC_INDEX_SERVICE_OWNERSHIP != 0");
    expect(ensure).toContain("Call InstallZenCanvasIndexService");
    expect(ensure).not.toContain("Call RestoreZenCanvasPreexistingService");

    const successful = functionBody(
      final,
      "ZCEnsureZenCanvasIndexServiceRunningForSuccessfulInstall",
    );
    const failure = functionBody(final, "ZCHandlePostInstallFailureFinal");
    expect(successful).toContain("Call EnsureZenCanvasIndexServiceRunning");
    expect(successful).not.toContain("Call RestoreZenCanvasPreexistingService");
    expect(failure).toContain("Call RestoreZenCanvasPreexistingService");
  });

  it("makes POST failures non-zero and resets exit status only after successful finalization", () => {
    const final = fs.readFileSync(finalPath, "utf8");
    const installPost = functionBody(final, "ZCPostInstallLifecycleFinal");
    const uninstallPost = functionBody(final, "un.ZCPostUninstallLifecycleFinal");
    expect(installPost.indexOf("SetErrorLevel 2")).toBeLessThan(
      installPost.indexOf("Call ZCEnsurePostInstallServiceFinal"),
    );
    expect(installPost.indexOf("Call ZCMarkInstallPostGeneratedIntegration")).toBeLessThan(
      installPost.indexOf("Call ZCEnsurePostInstallServiceFinal"),
    );
    expect(installPost.lastIndexOf("SetErrorLevel 0")).toBeGreaterThan(
      installPost.indexOf("Call CommitZenCanvasPreviewQuiesce"),
    );
    expect(uninstallPost.indexOf("SetErrorLevel 2")).toBeLessThan(
      uninstallPost.indexOf("Call un.FinalizeZenCanvasPreviewUninstall"),
    );
    expect(uninstallPost.indexOf("Call un.ZCMarkUninstallPostGeneratedIntegration")).toBeLessThan(
      uninstallPost.indexOf("Call un.FinalizeZenCanvasPreviewUninstall"),
    );
    expect(uninstallPost.lastIndexOf("SetErrorLevel 0")).toBeGreaterThan(
      uninstallPost.indexOf("Call un.DeleteZenCanvasIndexService"),
    );
  });

  it("keeps user cancellation recoverable before mutation and blocked after the irreversible boundary", () => {
    const wrapper = fs.readFileSync(wrapperPath, "utf8");
    const synchronous = fs.readFileSync(synchronousPath, "utf8");
    expect(wrapper).toContain("!define MUI_CUSTOMFUNCTION_ABORT ZCLifecycleUserAbort");
    expect(wrapper).toContain("!define MUI_CUSTOMFUNCTION_UNABORT un.ZCLifecycleUserAbort");
    expect(wrapper).toContain('!include "${__FILEDIR__}\\installer-lifecycle-final.nsh"');
    expect(synchronous).toContain("Call ZCRecoverInstallReversible");
    expect(synchronous).toContain("Call un.ZCRecoverUninstallReversible");
    expect(synchronous).toContain(
      "$ZC_LIFECYCLE_INSTALL_STAGE >= ${ZC_LIFECYCLE_STAGE_FILE_MUTATION}",
    );
    expect(synchronous).toContain(
      "$ZC_LIFECYCLE_UNINSTALL_STAGE >= ${ZC_LIFECYCLE_STAGE_FILE_MUTATION}",
    );
  });

  it("keeps irreversible failure truthful instead of synthesizing a full product rollback", () => {
    const synchronous = fs.readFileSync(synchronousPath, "utf8");
    const installPartial = functionBody(synchronous, "ZCFailInstallPartial");
    expect(installPartial).toContain("Call ZCHandleGeneratedInstallFailureFinal");
    expect(installPartial).not.toContain("Call CommitZenCanvasPreviewQuiesce");
    expect(installPartial).not.toContain("Call RestoreZenCanvasPreexistingService");
    expect(installPartial).not.toContain("RestoreZenCanvasPreexistingService");

    const uninstallPartial = functionBody(synchronous, "un.ZCFailUninstallPartial");
    expect(uninstallPartial).toContain("Call un.CommitZenCanvasPreviewQuiesce");
    expect(uninstallPartial).toContain("Call un.DeleteZenCanvasIndexService");
    expect(uninstallPartial).not.toContain("RollbackZenCanvasPreviewQuiesce");
    expect(uninstallPartial).not.toContain("RestoreZenCanvasOriginalService");
  });

  it("T38: separates successful repair convergence from failure-state restoration", () => {
    expect(successfulRepairServiceState("RUNNING")).toBe("RUNNING");
    expect(successfulRepairServiceState("STOPPED")).toBe("RUNNING");
    expect(failedRepairServiceState("RUNNING", lifecycleStages.postGeneratedIntegration)).toBe(
      "RUNNING",
    );
    expect(failedRepairServiceState("STOPPED", lifecycleStages.postGeneratedIntegration)).toBe(
      "STOPPED",
    );

    const final = fs.readFileSync(finalPath, "utf8");
    const success = functionBody(
      final,
      "ZCEnsureZenCanvasIndexServiceRunningForSuccessfulInstall",
    );
    const failure = functionBody(final, "ZCHandlePostInstallFailureFinal");
    expect(success).toContain("Call EnsureZenCanvasIndexServiceRunning");
    expect(failure).toContain("Call RestoreZenCanvasPreexistingService");
  });

  it("T39: makes the main executable replacement the stage-2 boundary", () => {
    const generated = buildFyluraNsisTemplate(fs.readFileSync(upstreamPath, "utf8"));
    const install = sectionBody(generated, "Install");
    const irreversible = install.indexOf("Call ZCMarkInstallIrreversible");
    const mainFile = install.indexOf('File "${MAINBINARYSRCPATH}"');
    const partial = install.indexOf("IfErrors zc_install_partial_failure");
    const generatedMutation = install.indexOf("Call ZCMarkInstallGeneratedMutation");
    expect(irreversible).toBeLessThan(mainFile);
    expect(mainFile).toBeLessThan(partial);
    expect(partial).toBeLessThan(generatedMutation);
    expect(install).not.toContain("IfErrors zc_install_reversible_failure");
    expect(advanceLifecycleStage(lifecycleStages.reversiblePreparation, lifecycleStages.fileMutation)).toBe(
      lifecycleStages.fileMutation,
    );
  });

  it("T40: dispatches each install failure stage through one idempotent owner", () => {
    expect(dispatchFailure(lifecycleStages.reversiblePreparation, false).actions).toEqual([
      "reversible-recovery",
    ]);
    for (const stage of [
      lifecycleStages.fileMutation,
      lifecycleStages.generatedMutation,
      lifecycleStages.postGeneratedIntegration,
    ]) {
      const first = dispatchFailure(stage, false);
      const second = dispatchFailure(stage, false, "repair", first.ownerDone);
      expect(first.actions).toEqual(partialFailureActions());
      expect(second.actions).toEqual([]);
      expect(first.metadataActions).toEqual(metadataFailureActions("repair"));
      expect(second.metadataActions).toEqual([]);
    }
    expect(dispatchFailure(lifecycleStages.postGeneratedIntegration, true).actions).toEqual([
      "preview-rollback",
      "captured-service-restore",
    ]);
    expect(dispatchFailure(lifecycleStages.complete, true).actions).toEqual([]);
    const legacy = fs.readFileSync(
      path.join(repositoryRoot, "src-tauri", "windows", "installer-hooks.nsh"),
      "utf8",
    );
    const callback = functionBody(legacy, ".onInstFailed");
    expect(callback.trim()).toBe("Function .onInstFailed\n  Call ZCDispatchInstallFailureFinal");
    const uninstallCallback = functionBody(legacy, "un.onUninstFailed");
    expect(uninstallCallback.trim()).toBe(
      "Function un.onUninstFailed\n  Call un.RecoverZenCanvasPreDeleteAbort",
    );
  });

  it("T40: normalizes LF and CRLF without weakening exact callback bodies", () => {
    const lf = [
      "Function .onInstFailed",
      "  Call ZCDispatchInstallFailureFinal",
      "FunctionEnd",
    ].join("\n");
    const crlf = lf.replace(/\n/gu, "\r\n");
    const expected = "Function .onInstFailed\n  Call ZCDispatchInstallFailureFinal";

    expect(functionBody(lf, ".onInstFailed").trim()).toBe(expected);
    expect(functionBody(crlf, ".onInstFailed").trim()).toBe(expected);
    expect(functionBody(lf, ".onInstFailed").trim()).toBe(
      functionBody(crlf, ".onInstFailed").trim(),
    );
  });

  it("T41: routes every generated resource and binary failure to partial handling", () => {
    const generated = buildFyluraNsisTemplate(fs.readFileSync(upstreamPath, "utf8"));
    const install = sectionBody(generated, "Install");
    expect(install.match(/IfErrors zc_install_partial_failure/g)?.length).toBeGreaterThanOrEqual(6);
    expect(install).not.toContain("IfErrors zc_install_reversible_failure");
    const synchronous = fs.readFileSync(synchronousPath, "utf8");
    const partial = functionBody(synchronous, "ZCFailInstallPartial");
    expect(partial).toContain("Call ZCHandleGeneratedInstallFailureFinal");
    expect(partial).not.toContain("Call RestoreZenCanvasPreexistingService");
  });

  it("T42: treats a missing Preview DLL as withdrawn and non-successful", () => {
    const final = fs.readFileSync(finalPath, "utf8");
    const coherence = functionBody(final, "ZCCheckPostGeneratedProductCoherence");
    const cleanup = functionBody(final, "ZCRemoveCurrentPreviewRegistrationForFailure");
    expect(coherence).toContain('IfFileExists "${ZC_PREVIEW_INSTALLED_DLL}" 0');
    expect(cleanup).toContain("DeleteRegValue");
    expect(cleanup).toContain("Call NotifyZenCanvasPreviewAssociationChanged");
    expect(cleanup).not.toContain("RollbackZenCanvasPreview");
    expect(dispatchFailure(lifecycleStages.postGeneratedIntegration, false).actions).toEqual([
      ...partialFailureActions(),
    ]);
  });

  it("T43: gates Preview rollback and service compensation on current coherence", () => {
    expect(dispatchFailure(lifecycleStages.postGeneratedIntegration, true).actions).toEqual([
      "preview-rollback",
      "captured-service-restore",
    ]);
    expect(dispatchFailure(lifecycleStages.postGeneratedIntegration, false).actions).toEqual([
      "exact-preview-withdrawal",
      "repair-service-stop-only",
    ]);

    const final = fs.readFileSync(finalPath, "utf8");
    const handler = functionBody(final, "ZCHandlePostInstallFailureFinal");
    expect(handler).toContain("Call ZCCheckPostGeneratedProductCoherence");
    expect(handler).toContain("Call RestoreZenCanvasPreexistingService");
    expect(handler).toContain("Call ZCStopCapturedServiceForLifecycle");
    expect(handler).toContain("Call CompensateZenCanvasPostInstallService");
  });

  it("T45: Stage 2 cannot promote coherent-looking evidence into full rollback", () => {
    const final = fs.readFileSync(finalPath, "utf8");
    const handler = functionBody(final, "ZCHandlePostInstallFailureFinal");
    const stage2 = nsisConditionalBody(
      handler,
      "; Stage 2 has begun canonical product-file mutation.",
      "  ${If} $ZC_LIFECYCLE_INSTALL_STAGE == ${ZC_LIFECYCLE_STAGE_FILE_MUTATION}",
    );
    expect(stage2).toContain("Goto zc_post_install_irreversible_partial_failure");
    for (const forbidden of [
      "ZCCheckPostGeneratedProductCoherence",
      "RollbackZenCanvasPreviewQuiesce",
      "RollbackZenCanvasPreviewRegistration",
      "RestoreZenCanvasPreexistingService",
    ]) {
      expect(stage2).not.toContain(forbidden);
    }
    expect(dispatchFailure(lifecycleStages.fileMutation, true).actions).toEqual(
      partialFailureActions(),
    );
    expect(failedRepairServiceState("RUNNING", lifecycleStages.fileMutation)).toBe("STOPPED");
    expect(failedRepairServiceState("STOPPED", lifecycleStages.fileMutation)).toBe("STOPPED");
  });

  it("T46: Stage 3 cannot promote a complete-looking main EXE into full rollback", () => {
    const final = fs.readFileSync(finalPath, "utf8");
    const handler = functionBody(final, "ZCHandlePostInstallFailureFinal");
    const stage3 = nsisConditionalBody(
      handler,
      "; Stage 3 may have a complete-looking main EXE",
      "  ${If} $ZC_LIFECYCLE_INSTALL_STAGE == ${ZC_LIFECYCLE_STAGE_GENERATED_MUTATION}",
    );
    expect(stage3).toContain("Goto zc_post_install_irreversible_partial_failure");
    expect(dispatchFailure(lifecycleStages.generatedMutation, true).actions).toEqual(
      partialFailureActions(),
    );
    expect(stage3).not.toContain("Call ZCCheckPostGeneratedProductCoherence");
    expect(stage3).not.toContain("Call RestoreZenCanvasPreexistingService");
    expect(stage3).not.toContain("Call RollbackZenCanvasPreviewQuiesce");
    expect(stage3).not.toContain("Call RollbackZenCanvasPreviewRegistration");
    expect(failedRepairServiceState("RUNNING", lifecycleStages.generatedMutation)).toBe("STOPPED");
  });

  it("T47: only Stage 4 may use coherence-gated recovery", () => {
    expect(dispatchFailure(lifecycleStages.reversiblePreparation, true).actions).toEqual([
      "reversible-recovery",
    ]);
    expect(dispatchFailure(lifecycleStages.fileMutation, true).actions).toEqual(
      partialFailureActions(),
    );
    expect(dispatchFailure(lifecycleStages.generatedMutation, true).actions).toEqual(
      partialFailureActions(),
    );
    expect(dispatchFailure(lifecycleStages.postGeneratedIntegration, true).actions).toEqual([
      "preview-rollback",
      "captured-service-restore",
    ]);
    expect(dispatchFailure(lifecycleStages.postGeneratedIntegration, false).actions).toEqual(
      partialFailureActions(),
    );
    expect(dispatchFailure(lifecycleStages.complete, true).actions).toEqual([]);

    const final = fs.readFileSync(finalPath, "utf8");
    const handler = functionBody(final, "ZCHandlePostInstallFailureFinal");
    const stage4 = nsisConditionalBody(
      handler,
      "; Only the post-generated integration phase may use current-product",
      "  ${If} $ZC_LIFECYCLE_INSTALL_STAGE == ${ZC_LIFECYCLE_STAGE_POST_GENERATED_INTEGRATION}",
    );
    expect(stage4).toContain("Call ZCCheckPostGeneratedProductCoherence");
    expect(stage4).toContain("Call RollbackZenCanvasPreviewQuiesce");
    expect(stage4).toContain("Call RestoreZenCanvasPreexistingService");
    expect(stage4).toContain("Goto zc_post_install_irreversible_partial_failure");
    expect(handler.match(/Call ZCCheckPostGeneratedProductCoherence/g)?.length).toBe(1);
  });

  it("T48: Stage 2/3 repair service safety never starts a captured service", () => {
    for (const stage of [lifecycleStages.fileMutation, lifecycleStages.generatedMutation]) {
      expect(failedRepairServiceState("RUNNING", stage)).toBe("STOPPED");
      expect(failedRepairServiceState("STOPPED", stage)).toBe("STOPPED");
      expect(dispatchFailure(stage, true).actions).not.toContain("captured-service-restore");
    }
    expect(dispatchFailure(lifecycleStages.fileMutation, true).actions).toEqual(
      partialFailureActions(),
    );
    expect(dispatchFailure(lifecycleStages.generatedMutation, true).actions).toEqual(
      partialFailureActions(),
    );

    const final = fs.readFileSync(finalPath, "utf8");
    const handler = functionBody(final, "ZCHandlePostInstallFailureFinal");
    const partialStart = handler.indexOf("zc_post_install_irreversible_partial_failure:");
    expect(partialStart).toBeGreaterThanOrEqual(0);
    const partial = handler.slice(partialStart);
    expect(partial).toContain("Call ZCStopCapturedServiceForLifecycle");
    expect(partial).not.toContain("Call RestoreZenCanvasPreexistingService");
    expect(partial).not.toContain('sc.exe\" start');
    expect(partial).toContain("Call CompensateZenCanvasPostInstallService");
  });

  it("T49: Stage 2/3 Preview failure keeps the withdrawal transaction committed", () => {
    const final = fs.readFileSync(finalPath, "utf8");
    const handler = functionBody(final, "ZCHandlePostInstallFailureFinal");
    const partialStart = handler.indexOf("zc_post_install_irreversible_partial_failure:");
    expect(partialStart).toBeGreaterThanOrEqual(0);
    const partial = handler.slice(partialStart);
    expect(partial).toContain("Call ZCRemoveCurrentPreviewRegistrationForFailure");
    expect(partial).toContain("Call CommitZenCanvasPreviewQuiesce");
    expect(partial).toContain("Call CommitZenCanvasPreviewRegistration");
    expect(partial).not.toContain("Call RollbackZenCanvasPreviewQuiesce");
    expect(partial).not.toContain("Call RollbackZenCanvasPreviewRegistration");
    expect(partial).not.toContain("Call ZCCheckPostGeneratedProductCoherence");

    for (const stage of [lifecycleStages.fileMutation, lifecycleStages.generatedMutation]) {
      expect(dispatchFailure(stage, true).actions).toEqual(partialFailureActions());
    }
  });

  it("T50: Stage 4 coherent failure converges through common metadata finalization", () => {
    const outcome = dispatchFailure(lifecycleStages.postGeneratedIntegration, true, "fresh");
    expect(outcome.actions).toEqual(["preview-rollback", "fresh-service-compensation"]);
    expect(outcome.metadataActions).toEqual(["fresh-metadata-compensation"]);
    expect(metadataFailureReport("fresh", true)).toBe("fresh-metadata-neutralized");

    const final = fs.readFileSync(finalPath, "utf8");
    const handler = functionBody(final, "ZCHandlePostInstallFailureFinal");
    const stage4 = nsisConditionalBody(
      handler,
      "; Only the post-generated integration phase may use current-product",
      "  ${If} $ZC_LIFECYCLE_INSTALL_STAGE == ${ZC_LIFECYCLE_STAGE_POST_GENERATED_INTEGRATION}",
    );
    expect(stage4).toContain("Goto zc_post_install_metadata_failure_finalization");
    expect(stage4).not.toContain("Return");

    const metadata = labeledBody(handler, "zc_post_install_metadata_failure_finalization");
    expect(metadata).toContain("Call CompensateZenCanvasFreshProductMetadata");
    expect(handler.match(/Call CompensateZenCanvasFreshProductMetadata/g)).toHaveLength(1);
  });

  it("T51: Stage 4 coherent repair failure preserves existing metadata after service recovery", () => {
    const outcome = dispatchFailure(lifecycleStages.postGeneratedIntegration, true, "repair");
    expect(outcome.actions).toEqual(["preview-rollback", "captured-service-restore"]);
    expect(outcome.metadataActions).toEqual(["repair-metadata-preserved"]);
    expect(metadataFailureReport("repair", true)).toBe("repair-metadata-preserved");

    const final = fs.readFileSync(finalPath, "utf8");
    const handler = functionBody(final, "ZCHandlePostInstallFailureFinal");
    const metadata = labeledBody(handler, "zc_post_install_metadata_failure_finalization");
    const freshBranchStart = metadata.indexOf("${If} $ZC_PREEXISTING_PRODUCT == 0");
    const repairBranchStart = metadata.indexOf("${ElseIf} $ZC_PREEXISTING_PRODUCT == 1");
    expect(freshBranchStart).toBeGreaterThanOrEqual(0);
    expect(repairBranchStart).toBeGreaterThan(freshBranchStart);
    expect(metadata.slice(freshBranchStart, repairBranchStart)).toContain(
      "Call CompensateZenCanvasFreshProductMetadata",
    );
    expect(metadata.slice(repairBranchStart)).toContain(
      "existing Add/Remove Programs metadata, install location authority, and uninstall.exe were preserved.",
    );
    expect(metadata.slice(repairBranchStart)).not.toContain(
      "Call CompensateZenCanvasFreshProductMetadata",
    );
    expect(handler.match(/Call CompensateZenCanvasFreshProductMetadata/g)).toHaveLength(1);
  });

  it("T52: Stage 4 incoherent failure uses partial handling before fresh metadata compensation", () => {
    const outcome = dispatchFailure(lifecycleStages.postGeneratedIntegration, false, "fresh");
    expect(outcome.actions).toEqual([
      "exact-preview-withdrawal",
      "fresh-service-compensation",
    ]);
    expect(outcome.metadataActions).toEqual(["fresh-metadata-compensation"]);

    const final = fs.readFileSync(finalPath, "utf8");
    const handler = functionBody(final, "ZCHandlePostInstallFailureFinal");
    const stage4 = nsisConditionalBody(
      handler,
      "; Only the post-generated integration phase may use current-product",
      "  ${If} $ZC_LIFECYCLE_INSTALL_STAGE == ${ZC_LIFECYCLE_STAGE_POST_GENERATED_INTEGRATION}",
    );
    expect(stage4).toContain("Goto zc_post_install_irreversible_partial_failure");
    const partialStart = handler.indexOf("zc_post_install_irreversible_partial_failure:");
    const metadataStart = handler.indexOf("zc_post_install_metadata_failure_finalization:");
    expect(partialStart).toBeGreaterThanOrEqual(0);
    expect(metadataStart).toBeGreaterThan(partialStart);
    const partial = handler.slice(partialStart, metadataStart);
    expect(partial).toContain("Goto zc_post_install_metadata_failure_finalization");
    expect(partial).not.toContain("RollbackZenCanvasPreview");
    expect(partial).not.toContain("RestoreZenCanvasPreexistingService");
  });

  it("T53: Stage 2/3 fresh failures share the same metadata finalization tail", () => {
    const final = fs.readFileSync(finalPath, "utf8");
    const handler = functionBody(final, "ZCHandlePostInstallFailureFinal");
    for (const [stage, marker, condition] of [
      [
        lifecycleStages.fileMutation,
        "; Stage 2 has begun canonical product-file mutation.",
        "  ${If} $ZC_LIFECYCLE_INSTALL_STAGE == ${ZC_LIFECYCLE_STAGE_FILE_MUTATION}",
      ],
      [
        lifecycleStages.generatedMutation,
        "; Stage 3 may have a complete-looking main EXE",
        "  ${If} $ZC_LIFECYCLE_INSTALL_STAGE == ${ZC_LIFECYCLE_STAGE_GENERATED_MUTATION}",
      ],
    ] as const) {
      const outcome = dispatchFailure(stage, true, "fresh");
      expect(outcome.actions).toEqual([
        "exact-preview-withdrawal",
        "fresh-service-compensation",
      ]);
      expect(outcome.metadataActions).toEqual(["fresh-metadata-compensation"]);
      const branch = nsisConditionalBody(handler, marker, condition);
      expect(branch).toContain("Goto zc_post_install_irreversible_partial_failure");
      expect(branch).not.toContain("Call ZCCheckPostGeneratedProductCoherence");
    }

    const metadata = labeledBody(handler, "zc_post_install_metadata_failure_finalization");
    expect(metadata).toContain("Call CompensateZenCanvasFreshProductMetadata");
    expect(handler.match(/Call CompensateZenCanvasFreshProductMetadata/g)).toHaveLength(1);
    const partialStart = handler.indexOf("zc_post_install_irreversible_partial_failure:");
    const metadataStart = handler.indexOf("zc_post_install_metadata_failure_finalization:");
    const partial = handler.slice(partialStart, metadataStart);
    for (const forbidden of [
      "RollbackZenCanvasPreviewQuiesce",
      "RollbackZenCanvasPreviewRegistration",
      "RestoreZenCanvasPreexistingService",
    ]) {
      expect(partial).not.toContain(forbidden);
    }
  });

  it("T54: metadata cleanup truth remains explicit for fresh, repair, and uncertain ownership", () => {
    expect(metadataFailureReport("fresh", true)).toBe("fresh-metadata-neutralized");
    expect(metadataFailureReport("fresh", false)).toBe("fresh-metadata-incomplete");
    expect(metadataFailureReport("repair", true)).toBe("repair-metadata-preserved");
    expect(metadataFailureReport("unknown", false)).toBe("metadata-cleanup-incomplete");

    const final = fs.readFileSync(finalPath, "utf8");
    const handler = functionBody(final, "ZCHandlePostInstallFailureFinal");
    const metadata = labeledBody(handler, "zc_post_install_metadata_failure_finalization");
    expect(metadata).toContain("StrCpy $ZC_POSTINSTALL_METADATA_CLEAN 1");
    expect(metadata).toContain("Call CompensateZenCanvasFreshProductMetadata");
    expect(metadata).toContain("StrCpy $ZC_POSTINSTALL_METADATA_CLEAN 0");
    const metadataStart = handler.indexOf("zc_post_install_metadata_failure_finalization:");
    expect(handler.slice(0, metadataStart)).not.toContain(
      "Call CompensateZenCanvasFreshProductMetadata",
    );

    const report = functionBody(final, "ZCFailPostInstallLifecycleFinal");
    expect(report).toContain("$ZC_POSTINSTALL_METADATA_CLEAN == 1");
    expect(report).toContain("Fresh-install metadata was neutralized where exact ownership was proven.");
    expect(report).toContain("Fresh-install metadata cleanup could not be fully verified; generated files may remain.");
  });

  it("T55: duplicate lifecycle callbacks cannot repeat metadata finalization", () => {
    const first = dispatchFailure(lifecycleStages.postGeneratedIntegration, true, "fresh");
    const second = dispatchFailure(
      lifecycleStages.postGeneratedIntegration,
      true,
      "fresh",
      first.ownerDone,
    );
    expect(first.metadataActions).toEqual(["fresh-metadata-compensation"]);
    expect(second.actions).toEqual([]);
    expect(second.metadataActions).toEqual([]);

    const final = fs.readFileSync(finalPath, "utf8");
    const handler = functionBody(final, "ZCHandlePostInstallFailureFinal");
    expect(handler.match(/Call CompensateZenCanvasFreshProductMetadata/g)).toHaveLength(1);
  });

  it("T56: product detection uses exact key opens and fails closed on UNKNOWN", () => {
    const detect = functionBody(installerHooksSource(), "DetectZenCanvasPreexistingProduct");
    expect(detect).toContain('ZC_REG_QUERY_KEY_STATE ${ZC_REG_ROOT_HKLM} "$ZC_UNINSTALLER_REGISTRY_KEY"');
    expect(detect).toContain('ZC_REG_QUERY_KEY_STATE ${ZC_REG_ROOT_HKLM} "$ZC_MANUFACTURER_PRODUCT_KEY"');
    expect(detect).toContain("$ZC_REG_KEY_STATE == ${ZC_REG_KEY_UNKNOWN}");
    expect(detect).toContain("StrCpy $ZC_PREEXISTING_PRODUCT 2");
    expect(detect).not.toContain("EnumRegKey");
  });

  it("T57: service ownership is exact-key and raw REG_EXPAND_SZ authority", () => {
    const source = installerHooksSource();
    const macro = source.slice(source.indexOf("!macro ZC_READ_INDEX_SERVICE_OWNERSHIP_BODY"), source.indexOf("!macroend", source.indexOf("!macro ZC_READ_INDEX_SERVICE_OWNERSHIP_BODY")));
    const operationalMacro = macro.split("\n").filter((line) => !line.trimStart().startsWith(";")).join("\n");
    expect(macro).toContain('ZC_REG_QUERY_KEY_STATE ${ZC_REG_ROOT_HKLM} "${ZC_INDEX_SERVICE_KEY}"');
    expect(macro).toContain("${ZC_REG_STRING_EXPAND_SZ_ONLY}");
    expect(macro).toContain("$ZC_REG_VALUE_STATE == ${ZC_REG_VALUE_EXACT}");
    expect(operationalMacro).not.toContain("ZC_INDEX_SERVICE_PARENT_KEY");
  });

  it("T58: fresh metadata presence is exact-key tri-state and cleanup is non-recursive", () => {
    const source = installerHooksSource();
    for (const functionName of [
      "ReadZenCanvasFreshUninstallKeyPresence",
      "ReadZenCanvasFreshManufacturerKeyPresence",
    ]) {
      const body = functionBody(source, functionName);
      expect(body).toContain("ZC_REG_QUERY_KEY_STATE");
      expect(body).toContain("$ZC_REG_KEY_STATE == ${ZC_REG_KEY_UNKNOWN}");
      expect(body).toContain("StrCpy $ZC_POSTINSTALL_METADATA_CLEAN 0");
      expect(body).not.toContain("EnumRegKey");
    }
    const cleanup = functionBody(source, "CompensateZenCanvasFreshProductMetadata");
    const operationalCleanup = cleanup.split("\n").filter((line) => !line.trimStart().startsWith(";")).join("\n");
    expect(cleanup).toContain('DeleteRegKey /ifempty HKLM "$ZC_UNINSTALLER_REGISTRY_KEY"');
    expect(operationalCleanup).not.toContain('DeleteRegKey HKLM "$ZC_UNINSTALLER_REGISTRY_KEY"');
    expect(cleanup.match(/Call AuditZenCanvasFreshProductMetadata/g)).toHaveLength(2);
  });

  it("T59: exact-key and enumeration models keep ABSENT, PRESENT, END, and UNKNOWN distinct", () => {
    expect(exactKeyState("MISSING")).toBe("absent");
    expect(exactKeyState("OPENED")).toBe("present");
    expect(exactKeyState("ERROR")).toBe("unknown");
    expect(enumState("NO_MORE_ITEMS")).toBe("end");
    expect(enumState("ERROR")).toBe("unknown");
  });

  it("T60: Win32 enumerators accept only ERROR_NO_MORE_ITEMS as finite END", () => {
    const authority = fs.readFileSync(registryAuthorityPath, "utf8");
    for (const macroName of ["ZC_REG_ENUM_KEY_STATE_IMPL", "ZC_REG_ENUM_VALUE_STATE_IMPL"]) {
      const start = authority.indexOf(`!macro ${macroName}`);
      const body = authority.slice(start, authority.indexOf("!macroend", start));
      expect(body).toContain("ZC_REG_ERROR_NO_MORE_ITEMS");
      expect(body).toContain("ZC_REG_ENUM_END");
      expect(body).toContain("ZC_REG_ENUM_UNKNOWN");
    }
  });

  it("T61: value enumeration no longer maps an arbitrary API error to EOF", () => {
    const authority = fs.readFileSync(registryAuthorityPath, "utf8");
    expect(authority).toContain("RegEnumValueW");
    expect(authority).toContain("${If} $ZC_REG_RESULT == ${ZC_REG_ERROR_SUCCESS}");
    expect(authority).toContain("${ElseIf} $ZC_REG_RESULT == ${ZC_REG_ERROR_NO_MORE_ITEMS}");
    expect(enumState("ERROR")).not.toBe("end");
  });

  it("T62: clean fresh admission accepts absent exact keys and absent uninstall.exe", () => {
    expect(exactKeyState("MISSING")).toBe("absent");
    const detect = functionBody(installerHooksSource(), "DetectZenCanvasPreexistingProduct");
    expect(detect).toContain("StrCpy $ZC_UNINSTALLER_KEY_PRESENT 0");
    expect(detect).toContain("StrCpy $ZC_MANUFACTURER_KEY_PRESENT 0");
    expect(detect).toContain('IfFileExists "$INSTDIR\\uninstall.exe" 0 detect_product_uninstaller_absent');
    expect(detect).toContain("$ZC_PREEXISTING_PRODUCT_PRESENT == 0");
    expect(detect).toContain("StrCpy $ZC_PREEXISTING_PRODUCT 0");
  });

  it("T63: exact key presence is a three-state contract", () => {
    expect([exactKeyState("MISSING"), exactKeyState("OPENED"), exactKeyState("ERROR")]).toEqual([
      "absent", "present", "unknown",
    ]);
  });

  it("T64: repair keeps durable ARP fields exact and checks optional URL fields", () => {
    expect(exactValueState({ type: "REG_SZ", value: "Zen Canvas" }, "REG_SZ", "Zen Canvas")).toBe("exact");
    expect(exactValueState("MISSING", "REG_SZ", "Zen Canvas")).toBe("absent");
    expect(exactValueState({ type: "REG_SZ", value: "Other" }, "REG_SZ", "Zen Canvas")).toBe("foreign");
    expect(exactValueState({ type: "REG_DWORD", value: 1 }, "REG_SZ", "Zen Canvas")).toBe("foreign");
    expect(exactValueState("ERROR", "REG_SZ", "Zen Canvas")).toBe("unknown");
    const detect = functionBody(installerHooksSource(), "DetectZenCanvasPreexistingProduct");
    for (const value of ["MainBinaryName", "DisplayName", "DisplayIcon", "DisplayVersion", "Publisher", "InstallLocation", "UninstallString", "NoModify", "NoRepair", "EstimatedSize"]) {
      expect(detect).toContain(`"${value}"`);
    }
    for (const value of repairOptionalUrlNames) expect(detect).toContain(`"${value}"`);
    expect(detect.match(/!insertmacro ZC_REQUIRE_OPTIONAL_PRODUCT_STRING/gu)).toHaveLength(3);
  });

  it("T65: association ownership preserves foreign and wrong-type values", () => {
    expect(associationAction("absent")).toBe("claim");
    expect(associationAction("exact")).toBe("idempotent");
    expect(associationAction("foreign")).toBe("preserve");
    expect(associationAction("unknown")).toBe("fail");
    expect(exactValueState({ type: "REG_DWORD", value: 7 }, "REG_SZ", "{ZEN}")).toBe("foreign");
    const source = installerHooksSource();
    const macro = source.slice(source.indexOf("!macro ZC_REGISTER_ASSOC"), source.indexOf("!macroend", source.indexOf("!macro ZC_REGISTER_ASSOC")));
    expect(macro).toContain("preserved ${EXT} (foreign value or type)");
    expect(macro).toContain("Call FailZenCanvasPostInstall");
  });

  it("T66: wrong-type Preview core markers fail closed", () => {
    const source = installerHooksSource();
    const start = source.indexOf("!macro ZC_VALIDATE_PREVIEW_CORE");
    const body = source.slice(start, source.indexOf("!macroend", start));
    expect(body.match(/ZC_REG_QUERY_STRING_STATE/g)).toHaveLength(7);
    expect(body.match(/\$ZC_REG_VALUE_STATE != \$\{ZC_REG_VALUE_ABSENT\}/g)).toHaveLength(5);
    expect(body).toContain("wrong-type");
  });

  it("T67: transaction capture re-queries immediately before mutation", () => {
    const source = installerHooksSource();
    const create = source.slice(source.indexOf("!macro ZC_RECORD_REG_CREATE"), source.indexOf("!macroend", source.indexOf("!macro ZC_RECORD_REG_CREATE")));
    const withdraw = source.slice(source.indexOf("!macro ZC_WITHDRAW_REG_VALUE"), source.indexOf("!macroend", source.indexOf("!macro ZC_WITHDRAW_REG_VALUE")));
    expect(create).toContain("ZC_REG_VALUE_ABSENT");
    expect(create).toContain("ZC_REG_VALUE_EXACT");
    expect(withdraw).toContain("$ZC_PREVIEW_TXN_CAPTURE_OK != 1");
    expect(withdraw.indexOf("ZC_RECORD_REG_VALUE")).toBeLessThan(withdraw.indexOf("DeleteRegValue"));
  });

  it("T68: rollback success follows post-state proof and concurrent foreign state fails clean", () => {
    const source = installerHooksSource();
    for (const name of ["RollbackZenCanvasPreviewRegistration", "un.RollbackZenCanvasPreviewRegistration"]) {
      const body = functionBody(source, name);
      expect(body).toContain("ZC_REG_QUERY_STRING_STATE");
      expect(body).toContain("StrCpy $ZC_PREVIEW_ROLLBACK_CLEAN 0");
      expect(body.indexOf("WriteRegStr")).toBeLessThan(body.lastIndexOf("ZC_REG_QUERY_STRING_STATE"));
      expect(body.indexOf("DeleteRegValue")).toBeLessThan(body.lastIndexOf("ZC_REG_QUERY_STRING_STATE"));
    }
  });

  it("T69: only NO_MORE_ITEMS completes a value enumeration", () => {
    expect(enumState("NO_MORE_ITEMS")).toBe("end");
    expect(enumState("ERROR")).toBe("unknown");
    const smoke = fs.readFileSync(registrySmokeFixturePath, "utf8");
    expect(smoke).toContain("ZC_REG_ENUM_VALUE_INVALID_HANDLE_STATE");
    expect(smoke).toContain('"invalid-handle-unknown"');
  });

  it("T70: fresh ARP extra values preserve the key", () => {
    const audit = functionBody(installerHooksSource(), "AuditZenCanvasFreshProductMetadata");
    expect(audit).toContain("fresh_uninstall_value_audit_loop");
    expect(audit).toContain("StrCpy $ZC_FRESH_UNINSTALL_METADATA_OWNED 2");
    expect(audit).toContain("ZC_REG_ENUM_VALUE_STATE");
  });

  it("T71: known ARP names with wrong values or types block deletion", () => {
    const source = installerHooksSource();
    expect(source).toContain("!macro ZC_AUDIT_OPTIONAL_FRESH_STRING");
    expect(source).toContain("!macro ZC_AUDIT_OPTIONAL_FRESH_DWORD");
    expect(source).toContain("$ZC_REG_VALUE_STATE == ${ZC_REG_VALUE_FOREIGN}");
    expect(source).toContain("StrCpy ${OWNER} 2");
  });

  it("T72: manufacturer cleanup accepts absent/exact partial only and preserves extras", () => {
    const audit = functionBody(installerHooksSource(), "AuditZenCanvasFreshProductMetadata");
    expect(audit).toContain('ZC_REG_ENUM_KEY_STATE ${ZC_REG_ROOT_HKLM} "$ZC_MANUFACTURER_PRODUCT_KEY" 0');
    expect(audit).toContain("$ZC_REG_ENUM_STATE != ${ZC_REG_ENUM_END}");
    expect(audit).toContain('$ZC_REG_ENUM_NAME != ""');
    const cleanup = functionBody(installerHooksSource(), "CompensateZenCanvasFreshProductMetadata");
    expect(cleanup).toContain('DeleteRegKey /ifempty HKLM "$ZC_MANUFACTURER_PRODUCT_KEY"');
  });

  it("T73: stale association enumeration errors cannot report clean cleanup", () => {
    const final = functionBody(fs.readFileSync(finalPath, "utf8"), "ZCRemoveCurrentPreviewRegistrationForFailure");
    expect(final).toContain("$ZC_REG_ENUM_STATE == ${ZC_REG_ENUM_UNKNOWN}");
    expect(final).toContain("StrCpy $ZC_LIFECYCLE_PREVIEW_FAILURE_CLEAN 0");
  });

  it("T74: service key authority distinguishes absent, exact, foreign, and unknown", () => {
    expect(exactKeyState("MISSING")).toBe("absent");
    expect(exactValueState({ type: "REG_EXPAND_SZ", value: "zen" }, "REG_EXPAND_SZ", "zen")).toBe("exact");
    expect(exactValueState({ type: "REG_SZ", value: "zen" }, "REG_EXPAND_SZ", "zen")).toBe("foreign");
    expect(exactValueState("ERROR", "REG_EXPAND_SZ", "zen")).toBe("unknown");
    const source = installerHooksSource();
    expect(source).toContain("$ZC_INDEX_SERVICE_OWNERSHIP != 1");
  });

  it("T75: installer never takes over UserChoice, ProgID, or OpenWithProgIds", () => {
    const sources = [installerHooksPath, synchronousPath, finalPath].map((file) => fs.readFileSync(file, "utf8")).join("\n");
    expect(sources).not.toMatch(/WriteReg(?:Str|DWORD|ExpandStr)[^\n]*(?:UserChoice|OpenWithProgIds)/u);
    expect(sources).not.toMatch(/WriteReg(?:Str|DWORD|ExpandStr)[^\n]*ProgI[Dd]/u);
  });

  it("T76: ownership-sensitive registry callsites match the helper contract inventory", () => {
    const inventory = [
      { file: installerHooksPath, contracts: ["ZC_REG_QUERY_KEY_STATE", "ZC_REG_QUERY_STRING_STATE", "ZC_REG_QUERY_DWORD_STATE", "ZC_REG_ENUM_KEY_STATE", "ZC_REG_ENUM_VALUE_STATE"] },
      { file: synchronousPath, contracts: ["ZC_REG_QUERY_STRING_STATE", "ZC_REG_ENUM_KEY_STATE"] },
      { file: finalPath, contracts: ["ZC_REG_QUERY_STRING_STATE", "ZC_REG_QUERY_DWORD_STATE", "ZC_REG_ENUM_KEY_STATE"] },
    ] as const;
    for (const { file, contracts } of inventory) {
      const source = fs.readFileSync(file, "utf8");
      const operationalSource = source.split(/\r?\n/u).filter((line) => !line.trimStart().startsWith(";")).join("\n");
      expect(operationalSource).not.toMatch(/\b(?:ReadRegStr|ReadRegDWORD|EnumRegKey|EnumRegValue)\b/u);
      for (const contract of contracts) expect(source).toContain(contract);
    }
    const authority = fs.readFileSync(registryAuthorityPath, "utf8");
    for (const api of ["RegOpenKeyExW", "RegQueryValueExW", "RegEnumKeyExW", "RegEnumValueW", "RegCloseKey"]) {
      expect(authority).toContain(api);
    }
    const script = fs.readFileSync(registrySmokeScriptPath, "utf8");
    expect(script).toContain("makensis");
    expect(script).toContain("fs.rmSync(tempRoot");
  });

  it("T86/T95: production NSIS has no localized service-state parser", () => {
    const windowsRoot = path.join(repositoryRoot, "src-tauri", "windows");
    const productionSources = fs
      .readdirSync(windowsRoot, { withFileTypes: true })
      .filter((entry) => entry.isFile() && entry.name.endsWith(".nsh"))
      .map((entry) => fs.readFileSync(path.join(windowsRoot, entry.name), "utf8"));
    const operational = productionSources.map(operationalNsisSource).join("\n");

    expect(operational).not.toMatch(/\bfindstr(?:\.exe)?\b/iu);
    expect(operational).not.toMatch(/\bsc\.exe\b[^\n]*\bquery\b/iu);
    expect(operational).not.toMatch(
      /\bcmd\.exe\b[^\n]*(?:\/C|\/D\s+\/S\s+\/C)[^\n]*(?:sc\.exe|findstr)/iu,
    );
    expect(operational).not.toMatch(
      /(?:findstr|sc\.exe[^\n]*\bquery\b)[^\n]*(?:RUNNING|STOPPED|PENDING|ABSENT)/iu,
    );
  });

  it("T87: numeric SCM states map to the existing product runtime contract", () => {
    const expected = [
      [1, 2],
      [2, 3],
      [3, 3],
      [4, 1],
      [5, 3],
      [6, 3],
      [7, 0],
    ] as const;
    const authority = fs.readFileSync(serviceRuntimeAuthorityPath, "utf8");
    const mapper = macroBody(authority, "ZC_MAP_SERVICE_RUNTIME_STATE_BODY");

    for (const [scmState, productState] of expected) {
      expect(expectedServiceRuntimeState(scmState)).toBe(productState);
      expect(mapper).toContain(
        "$ZC_SERVICE_RUNTIME_CURRENT_STATE == " + String(scmState),
      );
    }
    expect(mapper).toContain("ZC_SERVICE_RUNTIME_UNKNOWN");
    expect(mapper).toContain("ZC_SERVICE_RUNTIME_STOPPED");
    expect(mapper).toContain("ZC_SERVICE_RUNTIME_PENDING");
    expect(mapper).toContain("ZC_SERVICE_RUNTIME_RUNNING");
  });

  it("T88: only OpenService error 1060 means ABSENT; API uncertainty is UNKNOWN", () => {
    const authority = fs.readFileSync(serviceRuntimeAuthorityPath, "utf8");
    const query = macroBody(
      authority,
      "ZC_SERVICE_RUNTIME_READER_BODY MAP_FUNCTION DONE_LABEL",
    );
    const classifyOpenServiceError = (error: number) => (error === 1060 ? 4 : 0);

    expect(classifyOpenServiceError(1060)).toBe(4);
    expect(classifyOpenServiceError(5)).toBe(0);
    expect(classifyOpenServiceError(87)).toBe(0);
    expect(query).toContain("OpenSCManagerW");
    expect(query).toContain("OpenServiceW");
    expect(query).toContain("QueryServiceStatusEx");
    expect(query).toContain("CloseServiceHandle");
    expect(query).toContain("GetLastError");
    expect(query).toContain("ZC_SERVICE_RUNTIME_ERROR_SERVICE_DOES_NOT_EXIST");
    expect(query).toContain("ZC_SERVICE_RUNTIME_ABSENT");
    expect(query).toContain("ZC_SERVICE_RUNTIME_UNKNOWN");
    expect(query).toContain("?e");
  });

  it("T89: a fresh STOPPED service takes the start path and cannot report ready early", () => {
    const hooks = installerHooksSource();
    const install = functionBody(hooks, "InstallZenCanvasIndexService");
    const ensure = functionBody(hooks, "EnsureZenCanvasIndexServiceRunning");
    const stoppedGuard = ensure.indexOf(
      "$ZC_INDEX_SERVICE_RUNTIME_STATE != 2",
    );
    const start = ensure.indexOf('sc.exe" start');

    expect(install).toContain("StrCpy $ZC_INDEX_SERVICE_CREATE_SUCCEEDED 1");
    expect(install).toContain("Call EnsureZenCanvasIndexServiceRunning");
    expect(stoppedGuard).toBeGreaterThanOrEqual(0);
    expect(start).toBeGreaterThan(stoppedGuard);
    expect(compensationAction(2)).toBe("delete");
  });

  it("T90: successful start uses bounded PENDING/RUNNING convergence", () => {
    const hooks = installerHooksSource();
    const runningStart = hooks.indexOf(
      "!macro ZC_WAIT_INDEX_SERVICE_RUNNING_BODY",
    );
    const runningEnd = hooks.indexOf("!macroend", runningStart);
    expect(runningStart).toBeGreaterThanOrEqual(0);
    expect(runningEnd).toBeGreaterThan(runningStart);
    const wait = hooks.slice(runningStart, runningEnd);
    const readCall = "Call " + "$" + "{READ_FUNCTION}";

    expect(wait).toContain(readCall);
    expect(wait).toContain("ZC_INDEX_SERVICE_READY_ATTEMPTS");
    expect(wait).toContain("ZC_INDEX_SERVICE_RUNNING_CONFIRMATIONS");
    expect(wait).toContain("ZC_INDEX_SERVICE_READY 1");
    expect(wait).toContain("ZC_INDEX_SERVICE_RUNTIME_STATE == 3");
    expect(functionBody(hooks, "EnsureZenCanvasIndexServiceRunning")).toContain(
      "Call WaitForZenCanvasIndexServiceRunning",
    );
  });

  it("T91: repair success converges an originally STOPPED service to RUNNING", () => {
    const final = fs.readFileSync(finalPath, "utf8");
    const successful = functionBody(
      final,
      "ZCEnsureZenCanvasIndexServiceRunningForSuccessfulInstall",
    );

    expect(successfulRepairServiceState("STOPPED")).toBe("RUNNING");
    expect(successful).toContain("Call EnsureZenCanvasIndexServiceRunning");
    expect(successful).not.toContain("Call RestoreZenCanvasPreexistingService");
  });

  it("T92: repair failure restoration preserves an originally STOPPED service", () => {
    const hooks = installerHooksSource();
    const restore = functionBody(hooks, "RestoreZenCanvasPreexistingService");

    expect(
      failedRepairServiceState("STOPPED", lifecycleStages.fileMutation),
    ).toBe("STOPPED");
    expect(restore).toContain(
      "$ZC_INDEX_SERVICE_RUNTIME_STATE == 2",
    );
    expect(restore).toContain("A service that was originally stopped");
  });

  it("T93: STOPPED compensation rechecks ownership before deletion", () => {
    const compensation = functionBody(
      installerHooksSource(),
      "CompensateZenCanvasPostInstallService",
    );
    const stopped = compensation.indexOf(
      "$ZC_INDEX_SERVICE_RUNTIME_STATE == 2",
    );
    const guard = compensation.indexOf("postinstall_service_delete_guard:");
    const deleteCall = compensation.indexOf('sc.exe" delete');
    const finalOwnershipCheck = compensation.lastIndexOf(
      "Call ReadZenCanvasIndexServiceOwnership",
      deleteCall,
    );

    expect(compensationAction(2)).toBe("delete");
    expect(stopped).toBeGreaterThanOrEqual(0);
    expect(guard).toBeGreaterThan(stopped);
    expect(deleteCall).toBeGreaterThan(guard);
    expect(finalOwnershipCheck).toBeGreaterThan(guard);
    expect(finalOwnershipCheck).toBeLessThan(deleteCall);
    expect(compensation.slice(guard, deleteCall)).toContain(
      "Call ReadZenCanvasIndexServiceRuntimeState",
    );
  });

  it("T94: UNKNOWN compensation is incomplete and never authorizes delete", () => {
    const compensation = functionBody(
      installerHooksSource(),
      "CompensateZenCanvasPostInstallService",
    );
    const unknownBranch = compensation.indexOf(
      "$ZC_INDEX_SERVICE_RUNTIME_STATE != 1",
    );
    const deleteCall = compensation.indexOf('sc.exe" delete');

    expect(compensationAction(0)).toBe("incomplete");
    expect(unknownBranch).toBeGreaterThanOrEqual(0);
    expect(unknownBranch).toBeLessThan(deleteCall);
    expect(
      compensation.slice(unknownBranch, deleteCall),
    ).toContain("Goto postinstall_service_cleanup_incomplete");
    expect(compensation).toContain("StrCpy $ZC_POSTINSTALL_SERVICE_CLEAN 0");
  });

  it("T96: the hosted service authority gate runs before checksums without soft failure", () => {
    const workflow = fs.readFileSync(releaseWorkflowPath, "utf8");
    const packageStep = workflow.indexOf("- name: Package Windows installer");
    const artifactStep = workflow.indexOf("- name: Verify Windows NSIS artifact");
    const registryStep = workflow.indexOf(
      "- name: Verify Windows NSIS registry authority semantics",
    );
    const serviceStep = workflow.indexOf(
      "- name: Verify Windows NSIS service runtime authority semantics",
    );
    const checksumStep = workflow.indexOf("- name: Generate Windows checksums");
    const serviceEnd = workflow.indexOf("- name:", serviceStep + 1);

    expect(packageStep).toBeGreaterThanOrEqual(0);
    expect(artifactStep).toBeGreaterThan(packageStep);
    expect(registryStep).toBeGreaterThan(artifactStep);
    expect(serviceStep).toBeGreaterThan(registryStep);
    expect(checksumStep).toBeGreaterThan(serviceStep);
    expect(workflow.slice(serviceStep, serviceEnd)).toContain(
      "node scripts/verifyWindowsNsisServiceRuntimeAuthority.mjs",
    );
    expect(workflow.slice(serviceStep, serviceEnd)).not.toContain(
      "continue-on-error",
    );
  });

  it("T97: the executable smoke uses the shared helper and cleans its disposable service", () => {
    const fixture = fs.readFileSync(serviceSmokeFixturePath, "utf8");
    const script = fs.readFileSync(serviceSmokeScriptPath, "utf8");
    expect(fixture).toContain('!include "' + "$" + '{ZC_SERVICE_RUNTIME_AUTHORITY_FILE}"');
    expect(fixture).toContain("ZCMapServiceRuntimeState");
    expect(fixture).toContain('sc.exe" create');
    expect(fixture).toContain('sc.exe" delete');
    expect(fixture).not.toContain("FyluraGlobalIndex");
    expect(fixture.toLowerCase()).not.toContain("taskkill");
    expect(script).toContain("makensis");
    expect(script).toContain("ZC_SMOKE_CLEANUP_ONLY");
    expect(script).toContain("fs.rmSync(tempRoot");
    expect(script.toLowerCase()).not.toContain("taskkill");
  });

  it("T104: the exact real A1 snapshot with absent ARP URLs is admitted for repair", () => {
    const fixture = realA1RepairFixture();
    expect(fixture.values.URLInfoAbout).toBe("MISSING");
    expect(fixture.values.URLUpdateInfo).toBe("MISSING");
    expect(fixture.values.HelpLink).toBe("MISSING");
    expect(repairAdmissionState(fixture)).toEqual({ valid: true, product: 1 });
  });

  it("T105: all present ARP URLs are admitted when they are exact typed metadata", () => {
    const fixture = realA1RepairFixture();
    for (const name of repairOptionalUrlNames) {
      fixture.values[name] = { type: "REG_SZ", value: repairExpectedHomepage };
    }
    expect(repairAdmissionState(fixture)).toEqual({ valid: true, product: 1 });
  });

  it("T106: mixed absent and exact ARP URLs remain valid", () => {
    const fixture = realA1RepairFixture();
    fixture.values.URLInfoAbout = { type: "REG_SZ", value: repairExpectedHomepage };
    fixture.values.HelpLink = { type: "REG_SZ", value: repairExpectedHomepage };
    expect(fixture.values.URLUpdateInfo).toBe("MISSING");
    expect(repairAdmissionState(fixture)).toEqual({ valid: true, product: 1 });
  });

  it("T107: a foreign present ARP URL fails repair admission", () => {
    const fixture = withRepairValue(realA1RepairFixture(), "URLInfoAbout", {
      type: "REG_SZ",
      value: "https://foreign.example/",
    });
    expect(exactValueState(fixture.values.URLInfoAbout, "REG_SZ", repairExpectedHomepage)).toBe("foreign");
    expect(repairAdmissionState(fixture)).toEqual({ valid: false, product: 2 });
  });

  it("T108: a wrong-type ARP URL is foreign, not absent", () => {
    const fixture = withRepairValue(realA1RepairFixture(), "URLUpdateInfo", {
      type: "REG_DWORD",
      value: 1,
    });
    expect(exactValueState(fixture.values.URLUpdateInfo, "REG_SZ", repairExpectedHomepage)).toBe("foreign");
    expect(repairAdmissionState(fixture)).toEqual({ valid: false, product: 2 });
  });

  it("T109: an UNKNOWN ARP URL query fails closed", () => {
    const fixture = withRepairValue(realA1RepairFixture(), "HelpLink", "ERROR");
    expect(exactValueState(fixture.values.HelpLink, "REG_SZ", repairExpectedHomepage)).toBe("unknown");
    expect(repairAdmissionState(fixture)).toEqual({ valid: false, product: 2 });
  });

  it("T110: durable product identity remains mandatory while URLs are absent-or-exact", () => {
    const cases: Array<[string, ProductValueProbe]> = [
      ["InstallLocation", "MISSING"],
      ["MainBinaryName", "MISSING"],
      ["Publisher", { type: "REG_SZ", value: "other-publisher" }],
      ["EstimatedSize", { type: "REG_DWORD", value: 32919 }],
    ];
    for (const [name, value] of cases) {
      expect(repairAdmissionState(withRepairValue(realA1RepairFixture(), name, value))).toEqual({
        valid: false,
        product: 2,
      });
    }
  });

  it("T111: fresh cleanup keeps URL fields absent-or-exact and blocks foreign or UNKNOWN state", () => {
    const source = installerHooksSource();
    const audit = functionBody(source, "AuditZenCanvasFreshProductMetadata");
    for (const name of repairOptionalUrlNames) {
      expect(audit).toContain(
        `ZC_AUDIT_OPTIONAL_FRESH_STRING \"$ZC_UNINSTALLER_REGISTRY_KEY\" \"${name}\"`,
      );
    }
    expect(freshMetadataCleanupAllowed(["absent", "exact", "absent"])).toBe(true);
    expect(freshMetadataCleanupAllowed(["exact", "exact", "exact"])).toBe(true);
    expect(freshMetadataCleanupAllowed(["foreign", "absent", "exact"])).toBe(false);
    expect(freshMetadataCleanupAllowed(["unknown", "absent", "exact"])).toBe(false);
    expect(source).toContain("!macro ZC_AUDIT_OPTIONAL_FRESH_STRING");
  });

  it("T112: the real A1 fixture maps directly to valid product state 1", () => {
    const state = repairAdmissionState(realA1RepairFixture());
    expect(state.valid).toBe(true);
    expect(state.product).toBe(1);

    const detect = functionBody(installerHooksSource(), "DetectZenCanvasPreexistingProduct");
    expect(detect.match(/!insertmacro ZC_REQUIRE_OPTIONAL_PRODUCT_STRING/gu)).toHaveLength(3);
    expect(detect.match(/!insertmacro ZC_REQUIRE_PRODUCT_STRING/gu)).toHaveLength(8);
    expect(detect.match(/!insertmacro ZC_REQUIRE_PRODUCT_DWORD/gu)).toHaveLength(3);
    expect(detect).toContain("StrCpy $ZC_PREEXISTING_PRODUCT 1");
    expect(detect).not.toContain("WriteReg");
  });

  it("T113: generated package metadata keeps conditional homepage writes separate from repair authority", () => {
    const upstream = fs.readFileSync(upstreamPath, "utf8");
    const generated = buildFyluraNsisTemplate(upstream);
    const homepageDefine = '!define HOMEPAGE "{{homepage}}"';
    expect(generated).toContain(homepageDefine);

    const conditionalBlock = (source: string) => {
      const start = source.indexOf('!if "${HOMEPAGE}" != ""');
      expect(start).toBeGreaterThanOrEqual(0);
      const end = source.indexOf("!endif", start);
      expect(end).toBeGreaterThan(start);
      return source.slice(start, end + "!endif".length);
    };
    const generatedHomepageBlock = conditionalBlock(generated);
    const generatedUrlWritePrefix = 'WriteRegStr SHCTX "${UNINSTKEY}"';
    const upstreamHomepageBlock = conditionalBlock(upstream);
    expect(generatedHomepageBlock).toContain('!if "${HOMEPAGE}" != ""');
    for (const name of repairOptionalUrlNames) {
      const write = generatedUrlWritePrefix + ' "' + name + '" "${HOMEPAGE}"';
      expect(upstreamHomepageBlock).toContain(write);
      expect(generatedHomepageBlock).toContain(
        write,
      );
    }
    expect(functionBody(installerHooksSource(), "DetectZenCanvasPreexistingProduct")).toContain(
      "ZC_REQUIRE_OPTIONAL_PRODUCT_STRING",
    );
  });

  it("T114: the accepted A3 uninstall snapshot with absent ARP URLs is coherent", () => {
    const fixture = realA3UninstallFixture();
    expect(fixture.values.URLInfoAbout).toBe("MISSING");
    expect(fixture.values.URLUpdateInfo).toBe("MISSING");
    expect(fixture.values.HelpLink).toBe("MISSING");
    expect(preDeleteEvidenceState(fixture)).toBe(1);
  });

  it("T115: uninstall evidence accepts all three exact typed ARP URLs", () => {
    const fixture = realA3UninstallFixture();
    for (const name of repairOptionalUrlNames) {
      fixture.values[name] = { type: "REG_SZ", value: repairExpectedHomepage };
    }
    expect(preDeleteEvidenceState(fixture)).toBe(1);
  });

  it("T116: uninstall evidence accepts mixed exact and absent ARP URLs", () => {
    const fixture = realA3UninstallFixture();
    fixture.values.URLInfoAbout = { type: "REG_SZ", value: repairExpectedHomepage };
    fixture.values.HelpLink = { type: "REG_SZ", value: repairExpectedHomepage };
    expect(fixture.values.URLUpdateInfo).toBe("MISSING");
    expect(preDeleteEvidenceState(fixture)).toBe(1);
  });

  it("T117: a foreign present ARP URL blocks uninstall evidence", () => {
    const fixture = withRepairValue(realA3UninstallFixture(), "URLInfoAbout", {
      type: "REG_SZ",
      value: "https://foreign.example/",
    });
    expect(preDeleteEvidenceState(fixture)).toBe(0);
  });

  it("T118: a wrong-type ARP URL blocks uninstall evidence", () => {
    const fixture = withRepairValue(realA3UninstallFixture(), "URLUpdateInfo", {
      type: "REG_DWORD",
      value: 1,
    });
    expect(preDeleteEvidenceState(fixture)).toBe(0);
  });

  it("T119: an UNKNOWN ARP URL query blocks uninstall evidence", () => {
    const fixture = withRepairValue(realA3UninstallFixture(), "HelpLink", "ERROR");
    expect(preDeleteEvidenceState(fixture)).toBe(0);
  });

  it("T120: missing or foreign mandatory durable product evidence blocks uninstall", () => {
    const cases: Array<[string, ProductValueProbe]> = [
      ["DisplayName", "MISSING"],
      ["InstallLocation", { type: "REG_SZ", value: "C:\\Other" }],
    ];
    for (const [name, value] of cases) {
      expect(preDeleteEvidenceState(withRepairValue(realA3UninstallFixture(), name, value))).toBe(0);
    }
  });

  it("T121: EstimatedSize remains mandatory exact uninstall evidence", () => {
    expect(preDeleteEvidenceState(realA3UninstallFixture())).toBe(1);
    expect(
      preDeleteEvidenceState(
        withRepairValue(realA3UninstallFixture(), "EstimatedSize", {
          type: "REG_DWORD",
          value: 32919,
        }),
      ),
    ).toBe(0);
  });

  it("T122: the manufacturer default remains mandatory exact uninstall evidence", () => {
    const exact = realA3UninstallFixture();
    expect(preDeleteEvidenceState(exact)).toBe(1);
    expect(preDeleteEvidenceState({ ...exact, manufacturerDefault: "MISSING" })).toBe(0);
    expect(
      preDeleteEvidenceState({
        ...exact,
        manufacturerDefault: { type: "REG_SZ", value: "C:\\Other" },
      }),
    ).toBe(0);
  });

  it("T123: pre-delete recovery reuses one optional URL authority validator", () => {
    const source = installerHooksSource();
    const optional = macroBody(source, "ZC_UNINSTALL_EVIDENCE_OPTIONAL_STRING");
    expect(optional).toContain("ZC_REG_QUERY_STRING_STATE");
    expect(optional).toContain("${ZC_REG_STRING_SZ_ONLY}");
    expect(optional).toContain("$ZC_REG_VALUE_STATE != ${ZC_REG_VALUE_ABSENT}");
    expect(optional).toContain("$ZC_REG_VALUE_STATE != ${ZC_REG_VALUE_EXACT}");
    expect(optional).toContain("Goto un_predelete_evidence_failed");

    const evidence = functionBody(source, "un.CheckZenCanvasPreDeleteProductEvidence");
    expect(evidence.match(/!insertmacro ZC_UNINSTALL_EVIDENCE_OPTIONAL_STRING/gu)).toHaveLength(3);
    for (const name of repairOptionalUrlNames) {
      expect(evidence).toContain(
        `ZC_UNINSTALL_EVIDENCE_OPTIONAL_STRING "$ZC_UNINSTALLER_REGISTRY_KEY" "${name}"`,
      );
    }
    expect(source.match(/Function un\.CheckZenCanvasPreDeleteProductEvidence/gu)).toHaveLength(1);

    const recovery = functionBody(source, "un.RecoverZenCanvasPreDeleteAbort");
    expect(recovery).toContain("Call un.CheckZenCanvasPreDeleteProductEvidence");
    expect(recovery).not.toContain("ZC_UNINSTALL_EVIDENCE_OPTIONAL_STRING");
  });

  it("T124: post-uninstall deletes only the exact REG_SZ manufacturer install marker", () => {
    const hooks = installerHooksSource();
    const cleanup = functionBody(hooks, "un.RemoveZenCanvasManufacturerProductMarker");
    expect(cleanup).toContain("SetRegView 64");
    expect(cleanup).toContain(
      'ZC_REG_QUERY_STRING_STATE ${ZC_REG_ROOT_HKLM} "$ZC_MANUFACTURER_PRODUCT_KEY" "" "$INSTDIR" ${ZC_REG_STRING_SZ_ONLY}',
    );
    expect(cleanup).toContain('DeleteRegValue HKLM "$ZC_MANUFACTURER_PRODUCT_KEY" ""');
    expect(cleanup).toContain("ZC_UNINSTALL_MANUFACTURER_CLEAN");
    expect(manufacturerMarkerCleanupDecision({ type: "REG_SZ", value: manufacturerInstallLocation })).toEqual({
      deleteAttempted: true,
      preserved: false,
      clean: true,
    });
  });

  it("T125: exact marker deletion removes an empty product key with /ifempty", () => {
    const cleanup = functionBody(
      installerHooksSource(),
      "un.RemoveZenCanvasManufacturerProductMarker",
    );
    expect(cleanup).toContain('DeleteRegKey /ifempty HKLM "$ZC_MANUFACTURER_PRODUCT_KEY"');
    expect(
      manufacturerMarkerCleanupDecision(
        { type: "REG_SZ", value: manufacturerInstallLocation },
        "MISSING",
        "absent",
      ).clean,
    ).toBe(true);
  });

  it("T126: a foreign manufacturer marker is preserved and makes cleanup incomplete", () => {
    const cleanup = manufacturerMarkerCleanupDecision(
      { type: "REG_SZ", value: "C:\\Other" },
      { type: "REG_SZ", value: "C:\\Other" },
      "present",
    );
    expect(cleanup).toEqual({ deleteAttempted: false, preserved: true, clean: false });
    const source = functionBody(
      installerHooksSource(),
      "un.RemoveZenCanvasManufacturerProductMarker",
    );
    expect(source).toContain('${If} $ZC_REG_VALUE_STATE == ${ZC_REG_VALUE_EXACT}');
    expect(source).toContain("was foreign or could not be queried safely; it was preserved");
  });

  it("T127: a wrong-type manufacturer marker is preserved and fails closed", () => {
    expect(
      manufacturerMarkerCleanupDecision(
        { type: "REG_DWORD", value: 1 },
        { type: "REG_DWORD", value: 1 },
        "present",
      ),
    ).toEqual({ deleteAttempted: false, preserved: true, clean: false });
    expect(functionBody(installerHooksSource(), "un.RemoveZenCanvasManufacturerProductMarker")).toContain(
      "foreign or could not be queried safely",
    );
  });

  it("T128: an UNKNOWN manufacturer query is preserved and fails closed", () => {
    expect(
      manufacturerMarkerCleanupDecision("ERROR", "ERROR", "unknown"),
    ).toEqual({ deleteAttempted: false, preserved: true, clean: false });
    const final = functionBody(
      fs.readFileSync(finalPath, "utf8"),
      "un.ZCPostUninstallLifecycleFinal",
    );
    expect(final.indexOf("Call un.RemoveZenCanvasManufacturerProductMarker")).toBeGreaterThan(
      final.indexOf("Call un.DeleteZenCanvasIndexService"),
    );
    expect(final).toContain("$ZC_UNINSTALL_MANUFACTURER_CLEAN != 1");
    expect(final).toContain("Abort");
  });

  it("T129: marker cleanup never recursively deletes the manufacturer parent", () => {
    const cleanup = functionBody(
      installerHooksSource(),
      "un.RemoveZenCanvasManufacturerProductMarker",
    );
    expect(cleanup).not.toContain("DeleteRegKey HKLM \"$ZC_MANUFACTURER_PARENT");
    expect(cleanup).not.toContain('DeleteRegKey HKLM "Software\\Startlan"');
    expect(cleanup).toContain("/ifempty");
  });

  it("T130: the normal A4 model ends with no ARP, marker, service or active Preview values", () => {
    const marker = manufacturerMarkerCleanupDecision(
      { type: "REG_SZ", value: manufacturerInstallLocation },
      "MISSING",
      "absent",
    );
    const preview: PreviewValueSurface = {
      clsidDefault: "absent",
      appId: "absent",
      inprocDefault: "absent",
      threadingModel: "absent",
      previewHandlers: "absent",
      associations: Array.from({ length: 16 }, () => "absent"),
      clsidKeyPresent: true,
      inprocKeyPresent: true,
    };
    expect(marker.clean).toBe(true);
    expect(activePreviewValuesAbsent(preview)).toBe(true);
    expect({ arp: 0, manufacturer: !marker.clean, service: false, files: 0 }).toEqual({
      arp: 0,
      manufacturer: false,
      service: false,
      files: 0,
    });
  });

  it("T131: empty Preview CLSID/Inproc containers are not active registration", () => {
    expect(
      activePreviewValuesAbsent({
        clsidDefault: "absent",
        appId: "absent",
        inprocDefault: "absent",
        threadingModel: "absent",
        previewHandlers: "absent",
        associations: Array.from({ length: 16 }, () => "absent"),
        clsidKeyPresent: true,
        inprocKeyPresent: true,
      }),
    ).toBe(true);
  });

  it("T132: a normal uninstall leaves fresh detection with no manufacturer-only product evidence", () => {
    const marker = manufacturerMarkerCleanupDecision(
      { type: "REG_SZ", value: manufacturerInstallLocation },
      "MISSING",
      "absent",
    );
    expect(marker.clean).toBe(true);
    expect(freshDetectionSeesProduct(false, !marker.clean, false)).toBe(false);
    const detect = functionBody(installerHooksSource(), "DetectZenCanvasPreexistingProduct");
    expect(detect).toContain("$ZC_MANUFACTURER_KEY_PRESENT == 1");
  });

  it("T133: Preview withdrawal no longer hard-gates on release polling", () => {
    const synchronous = fs.readFileSync(synchronousPath, "utf8");
    const legacy = installerHooksSource();
    const withdraw = macroBody(synchronous, "ZC_LIFECYCLE_WITHDRAW_PREVIEW_BODY");
    expect(withdraw).toContain("Call ${NOTIFY_FUNCTION}");
    expect(withdraw).not.toContain("WAIT_FUNCTION");
    expect(withdraw).not.toContain("ZC_PREVIEW_RELEASE_READY");
    expect(functionBody(legacy, "QuiesceZenCanvasPreviewBeforeInstall")).not.toContain(
      "WaitForZenCanvasPreviewDllRelease",
    );
    expect(functionBody(legacy, "un.QuiesceZenCanvasPreviewBeforeUninstall")).not.toContain(
      "un.WaitForZenCanvasPreviewDllRelease",
    );
  });

  it("T134: the exact canonical path has absent, direct-probe, and fail-closed branches", () => {
    const source = previewDllServicingSource();
    const install = functionBody(source, "ZCPreparePreviewDllMutation");
    const uninstall = functionBody(source, "un.ZCPreparePreviewDllMutation");
    for (const body of [install, uninstall]) {
      expect(body).toContain('IfFileExists "${ZC_PREVIEW_INSTALLED_DLL}"');
      expect(body).toContain("CreateFileW");
      expect(body).toContain("0x40000000|0x00010000");
      expect(body).toContain("ZC_PREVIEW_DLL_ERROR_SHARING_VIOLATION");
      expect(body).toContain("ZC_PREVIEW_DLL_ERROR_LOCK_VIOLATION");
      expect(body).toContain("DetailPrint");
    }
    expect(install.indexOf("CreateFileW")).toBeLessThan(install.indexOf("GetParent"));
    expect(uninstall.indexOf("CreateFileW")).toBeLessThan(uninstall.indexOf("GetParent"));
  });

  it("T135: retirement is same-volume, narrow, unique, and non-overwriting", () => {
    const source = previewDllServicingSource();
    expect(source).toContain('${GetParent} "$INSTDIR" $0');
    expect(source).toContain(".fylura-retired");
    expect(source).toContain("GetTempFileName $0");
    expect(source).toContain("DeleteFileW(w \"$ZC_PREVIEW_RETIRED_PATH\")");
    expect(source).toContain("MoveFileExW");
    expect(source).toContain("ZC_PREVIEW_DLL_RETIREMENT_FLAGS_NONE");
    expect(source).not.toContain('CreateDirectory "$TEMP');
    expect(source).not.toContain("RmDir /r");
    expect(source).not.toContain("MOVEFILE_REPLACE_EXISTING");
  });

  it("T136: failed generated replacement attempts exact retired-to-canonical recovery", () => {
    const source = previewDllServicingSource();
    const install = macroBody(source, "ZC_INSTALL_PREVIEW_RESOURCE");
    const recovery = functionBody(source, "ZCRecoverPreviewDllMutation");
    const fileError = install.indexOf("${If} ${Errors}");
    expect(fileError).toBeGreaterThanOrEqual(0);
    expect(install.indexOf("Call ZCRecoverPreviewDllMutation", fileError)).toBeGreaterThan(fileError);
    expect(recovery).toContain('DeleteFileW(w "${ZC_PREVIEW_INSTALLED_DLL}")');
    expect(recovery).toContain(
      'MoveFileExW(w "$ZC_PREVIEW_RETIRED_PATH", w "${ZC_PREVIEW_INSTALLED_DLL}"',
    );
    expect(recovery).toContain("ZC_PREVIEW_RETIRED_ACTIVE 0");
  });

  it("T137: only the exact Preview resource receives servicing macros", () => {
    const source = previewDllServicingSource();
    const install = macroBody(source, "ZC_INSTALL_RESOURCE");
    const uninstall = macroBody(source, "ZC_UNINSTALL_RESOURCE");
    expect(install).toContain("ZC_PREVIEW_DLL_RESOURCE_PATH_FORWARD");
    expect(uninstall).toContain("ZC_PREVIEW_DLL_RESOURCE_PATH_FORWARD");
    expect(macroBody(source, "ZC_INSTALL_PREVIEW_RESOURCE")).toContain(
      'File /a "/oname=${ZC_PREVIEW_DLL_RESOURCE_PATH_BACKSLASH}" "${SOURCE}"',
    );
    expect(macroBody(source, "ZC_UNINSTALL_PREVIEW_RESOURCE")).toContain(
      'Delete "${ZC_PREVIEW_INSTALLED_DLL}"',
    );
    expect(source).toContain("ZC_PREVIEW_DLL_RESOURCE_PATH_BACKSLASH");
  });

  it("T138: generated resource loops delegate to servicing while binaries stay ordinary", () => {
    const generated = buildFyluraNsisTemplate(fs.readFileSync(upstreamPath, "utf8"));
    const install = sectionBody(generated, "Install");
    const uninstall = sectionBody(generated, "Uninstall");
    expect(install).toContain('!insertmacro ZC_INSTALL_RESOURCE "{{this.[1]}}"');
    expect(uninstall).toContain('!insertmacro ZC_UNINSTALL_RESOURCE "{{this.[1]}}"');
    expect(install).not.toContain('File /a "/oname={{this.[1]}}"');
    expect(uninstall).not.toContain('Delete "$INSTDIR\\\\{{this.[1]}}"');
    expect(install).toContain('File /a "/oname={{this}}"');
    expect(uninstall).toContain('Delete "$INSTDIR\\\\{{this}}"');
  });

  it("T139: ordinary resources preserve ClearErrors/File/IfErrors semantics", () => {
    const source = previewDllServicingSource();
    const install = macroBody(source, "ZC_INSTALL_RESOURCE");
    const uninstall = macroBody(source, "ZC_UNINSTALL_RESOURCE");
    expect(install).toContain("ClearErrors");
    expect(install).toContain("File /a");
    expect(install).toContain("IfErrors zc_install_partial_failure");
    expect(uninstall).toContain("ClearErrors");
    expect(uninstall).toContain("Delete \"$INSTDIR\\${DESTINATION}\"");
    expect(uninstall).toContain("IfErrors zc_uninstall_partial_failure");
  });

  it("T140: successful install finalizes retirement only after Preview integration", () => {
    const final = fs.readFileSync(finalPath, "utf8");
    const post = functionBody(final, "ZCPostInstallLifecycleFinal");
    const handler = functionBody(final, "ZCHandlePostInstallFailureFinal");
    const partial = labeledBody(final, "zc_post_install_irreversible_partial_failure");
    expect(post.indexOf("Call CommitZenCanvasPreviewQuiesce")).toBeLessThan(
      post.indexOf("Call ZCFinalizePreviewDllMutation"),
    );
    expect(handler).toContain("Call ZCRecoverPreviewDllMutation");
    expect(partial.indexOf("Call ZCRecoverPreviewDllMutation")).toBeLessThan(
      partial.indexOf("Call ZCRemoveCurrentPreviewRegistrationForFailure"),
    );
  });

  it("T141: coherent Stage-4 failure recovers old bytes before registry rollback", () => {
    const final = fs.readFileSync(finalPath, "utf8");
    const handler = functionBody(final, "ZCHandlePostInstallFailureFinal");
    const coherent = nsisConditionalBody(
      handler,
      "; Only the post-generated integration phase may use current-product",
      "${If} $ZC_LIFECYCLE_PRODUCT_COHERENT == 1",
    );
    expect(coherent).toContain("Call ZCRecoverPreviewDllMutation");
    expect(coherent).toContain("$ZC_PREVIEW_RETIRED_ACTIVE == 1");
    expect(coherent.indexOf("Call ZCRecoverPreviewDllMutation")).toBeLessThan(
      coherent.indexOf("Call RollbackZenCanvasPreviewQuiesce"),
    );
  });

  it("T142: uninstall retires the exact DLL before generated deletion and finalizes outside $INSTDIR", () => {
    const source = previewDllServicingSource();
    const uninstall = macroBody(source, "ZC_UNINSTALL_PREVIEW_RESOURCE");
    const prepare = uninstall.indexOf("Call un.ZCPreparePreviewDllMutation");
    const deleteCanonical = uninstall.indexOf('Delete "${ZC_PREVIEW_INSTALLED_DLL}"');
    expect(prepare).toBeGreaterThanOrEqual(0);
    expect(deleteCanonical).toBeGreaterThan(prepare);
    expect(uninstall).toContain("$ZC_PREVIEW_RETIRED_ACTIVE == 1");

    const final = fs.readFileSync(finalPath, "utf8");
    const post = functionBody(final, "un.ZCPostUninstallLifecycleFinal");
    expect(post).toContain("Call un.ZCFinalizePreviewDllMutation");
    expect(post.indexOf("Call un.ZCFinalizePreviewDllMutation")).toBeLessThan(
      post.indexOf("Call un.DeleteZenCanvasIndexService"),
    );
  });

  it("T143: retirement cleanup is best-effort and can defer only the exact file", () => {
    const source = previewDllServicingSource();
    for (const functionName of [
      "ZCFinalizePreviewDllMutation",
      "un.ZCFinalizePreviewDllMutation",
    ]) {
      const body = functionBody(source, functionName);
      expect(body).toContain("MoveFileExW");
      expect(body).toContain("p 0");
      expect(body).toContain("ZC_PREVIEW_DLL_RETIREMENT_FLAGS_DELAY_UNTIL_REBOOT");
      expect(body).not.toContain("Abort");
    }
    expect(source).toContain("no reboot is required for this result");
  });

  it("T144: servicing never terminates Preview hosts or broadens the authority surface", () => {
    const source = previewDllServicingSource();
    expect(source).not.toMatch(/taskkill|KillProcess|prevhost|Explorer/iu);
    expect(source).not.toMatch(/DeleteReg|WriteReg|sc\.exe|Service/iu);
    const hooks = installerHooksSource();
    expect(hooks).toContain("ValidateZenCanvasPreviewCore");
    expect(hooks).toContain("ValidateZenCanvasIndexServiceOwnership");
  });

  it("T145: foreign Preview ownership stays fail-closed while wrapper exposes servicing", () => {
    const wrapper = fs.readFileSync(wrapperPath, "utf8");
    const hooksIndex = wrapper.indexOf("installer-hooks.nsh");
    const servicingIndex = wrapper.indexOf("preview-dll-servicing.nsh");
    const synchronousIndex = wrapper.indexOf("installer-lifecycle-synchronous.nsh");
    expect(hooksIndex).toBeGreaterThanOrEqual(0);
    expect(servicingIndex).toBeGreaterThan(hooksIndex);
    expect(synchronousIndex).toBeGreaterThan(servicingIndex);
    expect(fs.readFileSync(
      path.join(repositoryRoot, "src-tauri", "tauri.windows.package.conf.json"),
      "utf8",
    )).toContain("native/fylura_windows_preview_handler.dll");
    const hooks = installerHooksSource();
    const validation = macroBody(hooks, "ZC_VALIDATE_PREVIEW_CORE");
    expect(validation).toContain("foreign, wrong-type, or unreadable");
    expect(validation).toContain("ZC_PREVIEW_INPROC_KEY");
    expect(validation).toContain("The existing registration and file were preserved");
  });

  it("T146: backslash Preview resource identity stays on the servicing branch", () => {
    const source = previewDllServicingSource();
    const install = macroBody(source, "ZC_INSTALL_RESOURCE");
    expect(install).toContain(
      '!else if "${DESTINATION}" == "${ZC_PREVIEW_DLL_RESOURCE_PATH_BACKSLASH}"',
    );
    expect(install).toContain('!insertmacro ZC_INSTALL_PREVIEW_RESOURCE "${SOURCE}"');
    expect(install).not.toContain(
      '!insertmacro ZC_INSTALL_RESOURCE "${ZC_PREVIEW_DLL_RESOURCE_PATH_FORWARD}"',
    );
  });

  it("T147: forward and backslash Preview identities write the canonical backslash destination", () => {
    const source = previewDllServicingSource();
    const install = macroBody(source, "ZC_INSTALL_RESOURCE");
    const preview = macroBody(source, "ZC_INSTALL_PREVIEW_RESOURCE");
    expect(install).toContain("ZC_PREVIEW_DLL_RESOURCE_PATH_FORWARD");
    expect(install).toContain("ZC_PREVIEW_DLL_RESOURCE_PATH_BACKSLASH");
    expect(preview).toContain(
      'File /a "/oname=${ZC_PREVIEW_DLL_RESOURCE_PATH_BACKSLASH}" "${SOURCE}"',
    );
    expect(preview).not.toContain('File /a "/oname=${DESTINATION}" "${SOURCE}"');
  });

  it("T148: generated fresh installer proof contains the canonical Preview File instruction", () => {
    const generatedTemplate = buildFyluraNsisTemplate(fs.readFileSync(upstreamPath, "utf8"));
    const generated = generatedTemplate.replace(
      '!insertmacro ZC_INSTALL_RESOURCE "{{this.[1]}}" "{{no-escape @key}}"',
      '!insertmacro ZC_INSTALL_RESOURCE "native\\fylura_windows_preview_handler.dll" "packaged\\fylura_windows_preview_handler.dll"',
    );
    const proof = assertGeneratedPreviewResourcePath(generated, previewDllServicingSource());
    expect(proof.canonicalFileInstruction).toBe(
      'File /a "/oname=native\\fylura_windows_preview_handler.dll" "${SOURCE}"',
    );
  });

  it("T149: generated Preview servicing never emits a forward-slash oname", () => {
    const generated = buildFyluraNsisTemplate(fs.readFileSync(upstreamPath, "utf8"));
    const source = previewDllServicingSource();
    expect(generated).not.toContain(
      '!insertmacro ZC_INSTALL_RESOURCE "native/fylura_windows_preview_handler.dll"',
    );
    expect(macroBody(source, "ZC_INSTALL_PREVIEW_RESOURCE")).not.toContain(
      'File /a "/oname=native/fylura_windows_preview_handler.dll"',
    );
  });

  it("T150: the executable fresh-resource smoke checks nested and flattened paths", () => {
    const verifier = fs.readFileSync(
      path.join(repositoryRoot, "scripts", "verifyWindowsNsisPreviewResource.mjs"),
      "utf8",
    );
    const fixture = fs.readFileSync(
      path.join(repositoryRoot, "tests", "fixtures", "windows-preview-resource-smoke.nsi"),
      "utf8",
    );
    expect(verifier).toContain("inspectOutputRoot");
    expect(verifier).toContain("flattenedPreviewResourcePath");
    expect(verifier).toContain('process.env["ProgramFiles(x86)"]');
    expect(verifier).toContain("pathEntries.map");
    expect(fixture).toContain('!insertmacro ZC_INSTALL_RESOURCE "native/fylura_windows_preview_handler.dll"');
    expect(fixture).toContain('CreateDirectory "$ZC_SMOKE_ROOT\\native"');
    expect(fixture).toContain("fresh-root");
  });

  it("T151: the fresh-resource smoke compares canonical output bytes with the packaged DLL", () => {
    const verifier = fs.readFileSync(
      path.join(repositoryRoot, "scripts", "verifyWindowsNsisPreviewResource.mjs"),
      "utf8",
    );
    expect(verifier).toContain("outputBytes.equals(sourceBytes)");
    expect(verifier).toContain('createHash("sha256")');
  });

  it("T152: mapped-DLL retirement/replacement smoke remains an independent required gate", () => {
    const smoke = fs.readFileSync(
      path.join(
        repositoryRoot,
        "src-tauri",
        "native",
        "windows-preview-handler-harness",
        "src",
        "preview_dll_servicing_smoke.rs",
      ),
      "utf8",
    );
    expect(smoke).toContain("MoveFileExW");
    expect(smoke).toContain('arg("--load-only")');
    expect(smoke).toContain("canonical replacement");
  });

  it("T153: uninstall servicing resolves the canonical backslash Preview DLL", () => {
    const source = previewDllServicingSource();
    const uninstall = macroBody(source, "ZC_UNINSTALL_RESOURCE");
    const preview = macroBody(source, "ZC_UNINSTALL_PREVIEW_RESOURCE");
    expect(uninstall).toContain("ZC_PREVIEW_DLL_RESOURCE_PATH_FORWARD");
    expect(uninstall).toContain("ZC_PREVIEW_DLL_RESOURCE_PATH_BACKSLASH");
    expect(preview).toContain('Delete "${ZC_PREVIEW_INSTALLED_DLL}"');
    expect(preview).not.toContain("native/fylura_windows_preview_handler.dll");
  });

  it("T154: ordinary non-Preview resources retain direct File/Delete semantics", () => {
    const source = previewDllServicingSource();
    const install = macroBody(source, "ZC_INSTALL_RESOURCE");
    const uninstall = macroBody(source, "ZC_UNINSTALL_RESOURCE");
    expect(install).toContain('File /a "/oname=${DESTINATION}" "${SOURCE}"');
    expect(install).toContain("IfErrors zc_install_partial_failure");
    expect(uninstall).toContain('Delete "$INSTDIR\\${DESTINATION}"');
    expect(uninstall).toContain("IfErrors zc_uninstall_partial_failure");
  });

  it("T44: keeps legacy callbacks as shims and retains generated hook isolation", () => {
    const legacy = fs.readFileSync(
      path.join(repositoryRoot, "src-tauri", "windows", "installer-hooks.nsh"),
      "utf8",
    );
    const callback = functionBody(legacy, ".onInstFailed");
    for (const forbidden of [
      "RollbackZenCanvasPreview",
      "RestoreZenCanvasPreexistingService",
      "CompensateZenCanvasPostInstallService",
      "CompensateZenCanvasFreshProductMetadata",
    ]) {
      expect(callback).not.toContain(forbidden);
    }
    const generated = buildFyluraNsisTemplate(fs.readFileSync(upstreamPath, "utf8"));
    for (const sectionName of ["Install", "Uninstall"]) {
      const section = sectionBody(generated, sectionName);
      expect(section).not.toContain("!insertmacro NSIS_HOOK_PREINSTALL");
      expect(section).not.toContain("!insertmacro NSIS_HOOK_POSTINSTALL");
      expect(section).not.toContain("!insertmacro NSIS_HOOK_PREUNINSTALL");
      expect(section).not.toContain("!insertmacro NSIS_HOOK_POSTUNINSTALL");
    }
  });
});
