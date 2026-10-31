import fs from "node:fs";
import path from "node:path";
import {
  buildFyluraNsisTemplate as buildBaseFyluraNsisTemplate,
  cleanWindowsNsisTemplate,
  generatedTemplatePath,
  TAURI_NSIS_UPSTREAM_BLOB_SHA,
  upstreamTemplatePath,
} from "./prepareWindowsNsisTemplate.mjs";
import { hardenWindowsNsisGeneratedMetadata } from "./hardenWindowsNsisGeneratedMetadata.mjs";
import { finalizeWindowsNsisLifecycleOrchestration } from "./finalizeWindowsNsisLifecycleOrchestration.mjs";
import { relocateWindowsNsisInstallerHooks } from "./relocateWindowsNsisInstallerHooks.mjs";

export {
  cleanWindowsNsisTemplate,
  generatedTemplatePath,
  TAURI_NSIS_UPSTREAM_BLOB_SHA,
  upstreamTemplatePath,
};

export function buildFyluraNsisTemplate(upstream) {
  const structural = buildBaseFyluraNsisTemplate(upstream);
  const metadataHardened = hardenWindowsNsisGeneratedMetadata(structural);
  const finalized = finalizeWindowsNsisLifecycleOrchestration(metadataHardened);
  return relocateWindowsNsisInstallerHooks(finalized);
}

export function prepareWindowsNsisTemplate() {
  const upstream = fs.readFileSync(upstreamTemplatePath, "utf8");
  const output = buildFyluraNsisTemplate(upstream);
  fs.mkdirSync(path.dirname(generatedTemplatePath), { recursive: true });
  fs.writeFileSync(generatedTemplatePath, output, "utf8");
  return generatedTemplatePath;
}
