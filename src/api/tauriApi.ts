import { fileManagementApi } from "./fileManagementApi";
import { automationApi } from "./automationApi";
import { aiApi } from "./aiApi";
import { analysisApi } from "./analysisApi";
import { cleanupApi } from "./cleanupApi";
import { contentApi } from "./contentApi";
import { dedupeApi } from "./dedupeApi";
import { globalSearchApi } from "./globalSearchApi";
import { libraryApi } from "./libraryApi";
import { operationApi } from "./operationApi";
import { organizationApi } from "./organizationApi";
import { runtimeApi } from "./runtimeApi";
import { rulesApi } from "./rulesApi";
import { scanApi } from "./scanApi";
import { settingsApi } from "./settingsApi";
import { windowApi } from "./windowApi";
export type {
  DedupeCompletePayload,
  DedupeProgressPayload,
  GlobalHotkeyErrorPayload,
  GlobalHotkeyStatus,
  MainWindowReadyRequest,
  ManagedScanEvent,
  ManagedScanRequest,
  ManagedScanSnapshotDto,
  ManagedScanStartDto,
  OperationProgressPayload,
  ScanBatchPayload,
  ScanProgressPayload,
  ScanRootDto,
  ScanRunDto,
  ScanSessionDto,
  ScanSummary,
  ScannedEntry,
  SearchWindowSnapshot,
  TauriSearchFileResult,
  WatcherReconciliationStatus
} from "./types";

export const tauriApi = {
  ...fileManagementApi,
  ...automationApi,
  ...libraryApi,
  ...organizationApi,
  ...globalSearchApi,
  ...scanApi,
  ...dedupeApi,
  ...analysisApi,
  ...operationApi,
  ...cleanupApi,
  ...rulesApi,
  ...contentApi,
  ...aiApi,
  ...settingsApi,
  ...windowApi,
  ...runtimeApi
};

export type TauriApi = typeof tauriApi;
