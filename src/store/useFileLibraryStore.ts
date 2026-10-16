import { create } from "zustand";
import { tauriApi } from "../api/tauriApi";
import type {
  DashboardStats,
  AIClassificationProgressPayload,
  FileLibraryFilters,
  FileQueryResult,
  FileRecord,
  LibraryFilter,
  LibraryScope,
  ScanRootSetting,
  RuleExecutionSummary
} from "../types/domain";
import { readableError } from "../utils/viewHelpers";
import { useAppStore } from "./useAppStore";

export const LIBRARY_PAGE_SIZE = 50;
export const ORGANIZE_QUEUE_PAGE_SIZE = 500;
export const ORGANIZE_QUEUE_MAX_FILES = 3000;
export const LIBRARY_SCOPE_STORAGE_KEY = "zc-library-scope";
const LIBRARY_SCOPE_STORAGE_VERSION = 1;
export const defaultLibraryScope: LibraryScope = { kind: "current_scan", roots: [] };

function browserLocalStorage(): Storage | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === "string");
}

function isLibraryScope(value: unknown): value is LibraryScope {
  if (!value || typeof value !== "object" || !("kind" in value)) return false;
  const scope = value as Partial<LibraryScope>;
  if (scope.kind === "all") return true;
  if (scope.kind === "roots") return isStringArray(scope.roots);
  if (scope.kind === "current_scan") {
    return isStringArray(scope.roots)
      && (!("scanSessionId" in scope) || typeof scope.scanSessionId === "string" || scope.scanSessionId === undefined);
  }
  return false;
}

export function readPersistedLibraryScope(): LibraryScope {
  const raw = browserLocalStorage()?.getItem(LIBRARY_SCOPE_STORAGE_KEY);
  if (!raw) return defaultLibraryScope;

  try {
    const parsed = JSON.parse(raw) as unknown;
    if (isLibraryScope(parsed)) return parsed;
    if (
      parsed && typeof parsed === "object"
      && "version" in parsed && parsed.version === LIBRARY_SCOPE_STORAGE_VERSION
      && "scope" in parsed && isLibraryScope(parsed.scope)
    ) {
      return parsed.scope;
    }
    return defaultLibraryScope;
  } catch {
    return defaultLibraryScope;
  }
}

function persistLibraryScope(scope: LibraryScope) {
  try {
    browserLocalStorage()?.setItem(
      LIBRARY_SCOPE_STORAGE_KEY,
      JSON.stringify({ version: LIBRARY_SCOPE_STORAGE_VERSION, scope })
    );
  } catch {
    // State remains authoritative for this session when storage is unavailable or full.
  }
}

function filtersForLibraryFilter(libraryFilter: LibraryFilter): FileLibraryFilters | undefined {
  return libraryFilter === "all" ? undefined : { libraryFilter };
}

export const emptyStats: DashboardStats = {
  totalFiles: 0,
  totalSize: 0,
  diskTotalSize: 0,
  diskFreeSize: 0,
  diskUsageRatio: 0,
  duplicateFiles: 0,
  largeFiles: 0,
  sensitiveFiles: 0,
  needsConfirmation: 0,
  byType: {},
  byLifecycle: {},
  lastScannedAt: null
};

export const emptyPage: FileQueryResult = {
  files: [],
  total: 0,
  limit: LIBRARY_PAGE_SIZE,
  offset: 0
};

export interface FileLibraryStore {
  scope: LibraryScope;
  stats: DashboardStats;
  libraryPage: FileQueryResult;
  organizeQueue: FileRecord[];
  organizeQueueTotal: number;
  organizeQueueTruncated: boolean;
  isLoadingOrganizeQueue: boolean;
  organizeQueueError: string;
  libraryFilter: LibraryFilter;
  selectedFileId: string;
  isClassifyingWithAI: boolean;
  aiClassificationStatus: string;
  aiClassificationProgress: AIClassificationProgressPayload | null;
  activeAIJobId: string | null;
  firstPageRequestId: number;
  statsRequestId: number;
  organizeQueueRequestId: number;
  setScope: (scope: LibraryScope) => void;
  adoptConfiguredRootsIfScopeEmpty: (roots: ScanRootSetting[]) => boolean;
  setCurrentScanScope: (roots: string[], scanSessionId?: string) => void;
  setLibraryFilter: (libraryFilter: LibraryFilter) => void;
  setLibraryPage: (page: FileQueryResult | ((current: FileQueryResult) => FileQueryResult)) => void;
  setSelectedFileId: (id: string) => void;
  loadStats: (scope?: LibraryScope) => Promise<void>;
  loadFirstPage: (query?: string, scope?: LibraryScope, libraryFilter?: LibraryFilter) => Promise<void>;
  loadOrganizeQueue: (scope?: LibraryScope) => Promise<void>;
  classifyCurrentScopeWithAI: (options?: AIClassificationRunOptions) => Promise<RuleExecutionSummary>;
  classifySelectedFileWithAI: (fileId?: string) => Promise<RuleExecutionSummary>;
  applyAIClassificationProgress: (progress: AIClassificationProgressPayload) => void;
  cancelAIClassification: () => Promise<void>;
  clearAIClassificationStatus: () => void;
  refresh: (query?: string) => Promise<void>;
}

export interface AIClassificationRunOptions {
  pendingOnly?: boolean;
  onlyUnclassified?: boolean;
  onlyLowConfidence?: boolean;
  limit?: number;
  force?: boolean;
  allowOverwriteUserCorrections?: boolean;
}

export const useFileLibraryStore = create<FileLibraryStore>((set, get) => ({
  scope: readPersistedLibraryScope(),
  stats: emptyStats,
  libraryPage: emptyPage,
  organizeQueue: [],
  organizeQueueTotal: 0,
  organizeQueueTruncated: false,
  isLoadingOrganizeQueue: false,
  organizeQueueError: "",
  libraryFilter: "all",
  selectedFileId: "",
  isClassifyingWithAI: false,
  aiClassificationStatus: "",
  aiClassificationProgress: null,
  activeAIJobId: null,
  firstPageRequestId: 0,
  statsRequestId: 0,
  organizeQueueRequestId: 0,
  setScope: (scope) => {
    persistLibraryScope(scope);
    set({ scope });
  },
  adoptConfiguredRootsIfScopeEmpty: (roots) => {
    const currentScope = get().scope;
    const hasConfiguredRoot = roots.some((root) => root.enabled && root.path.trim());
    if (!hasConfiguredRoot
      || currentScope.kind !== "current_scan"
      || currentScope.roots.length > 0
      || currentScope.scanSessionId) return false;

    get().setScope({ kind: "all" });
    return true;
  },
  setCurrentScanScope: (roots, scanSessionId) => {
    const scope: LibraryScope = {
      kind: "current_scan",
      roots,
      ...(scanSessionId ? { scanSessionId } : {})
    };
    persistLibraryScope(scope);
    set({ scope });
  },
  setLibraryFilter: (libraryFilter) => set({ libraryFilter }),
  setLibraryPage: (page) =>
    set((state) => ({
      libraryPage: typeof page === "function" ? page(state.libraryPage) : page
    })),
  setSelectedFileId: (selectedFileId) => set({ selectedFileId }),
  loadStats: async (scope = get().scope) => {
    const requestId = get().statsRequestId + 1;
    set({ statsRequestId: requestId });
    try {
      const stats = await tauriApi.getStatsSummary(scope);
      if (requestId !== get().statsRequestId) return;
      set({ stats });
    } catch (error) {
      if (requestId !== get().statsRequestId) return;
      set({ stats: emptyStats });
      useAppStore.getState().showError(readableError(error));
    }
  },
  loadFirstPage: async (query, scope = get().scope, libraryFilter = get().libraryFilter) => {
    const requestId = get().firstPageRequestId + 1;
    set({ firstPageRequestId: requestId });
    try {
      const page = await tauriApi.getPagedFiles(
        LIBRARY_PAGE_SIZE,
        0,
        query || undefined,
        scope,
        filtersForLibraryFilter(libraryFilter)
      );
      if (requestId !== get().firstPageRequestId) return;
      set((state) => ({
        libraryPage: page,
        selectedFileId: page.files.some((file) => file.id === state.selectedFileId)
          ? state.selectedFileId
          : page.files[0]?.id || ""
      }));
    } catch (error) {
      if (requestId !== get().firstPageRequestId) return;
      set({
        libraryPage: emptyPage,
        selectedFileId: ""
      });
      useAppStore.getState().showError(readableError(error));
    }
  },
  loadOrganizeQueue: async (scope = get().scope) => {
    const requestId = get().organizeQueueRequestId + 1;
    set({ isLoadingOrganizeQueue: true, organizeQueueError: "", organizeQueueRequestId: requestId });
    try {
      const files: FileRecord[] = [];
      let total = 0;
      let offset = 0;

      while (files.length < ORGANIZE_QUEUE_MAX_FILES) {
        const page = await tauriApi.getPagedFiles(ORGANIZE_QUEUE_PAGE_SIZE, offset, undefined, scope);
        total = page.total;
        files.push(...page.files.slice(0, ORGANIZE_QUEUE_MAX_FILES - files.length));

        if (!page.files.length || files.length >= total) break;
        offset += ORGANIZE_QUEUE_PAGE_SIZE;
      }

      if (requestId !== get().organizeQueueRequestId) return;
      set({
        organizeQueue: files,
        organizeQueueTotal: total,
        organizeQueueTruncated: total > ORGANIZE_QUEUE_MAX_FILES,
        isLoadingOrganizeQueue: false,
        organizeQueueError: ""
      });
    } catch (error) {
      if (requestId !== get().organizeQueueRequestId) return;
      set({
        organizeQueue: [],
        organizeQueueTotal: 0,
        organizeQueueTruncated: false,
        isLoadingOrganizeQueue: false,
        organizeQueueError: readableError(error)
      });
      useAppStore.getState().showError(readableError(error));
    }
  },
  classifyCurrentScopeWithAI: async (options) => {
    if (get().isClassifyingWithAI) {
      return { scanned: 0, updated: 0, skipped: 0, needsConfirmation: 0 };
    }
    set({ isClassifyingWithAI: true, aiClassificationStatus: "", aiClassificationProgress: null, activeAIJobId: null });
    try {
      const settings = await tauriApi.getAISettings();
      ensureAIReady(settings.enabled, settings.provider, settings.apiKey, settings.apiKeyConfigured);
      const summary = await tauriApi.classifyFilesWithAI(get().scope, options);
      await get().refresh(useAppStore.getState().searchQuery);
      await get().loadOrganizeQueue(get().scope);
      const message = aiClassificationSummaryMessage(summary);
      set({ aiClassificationStatus: message });
      useAppStore.getState().showSuccess(message);
      return summary;
    } catch (error) {
      const message = readableAIClassificationError(error);
      set({ aiClassificationStatus: message });
      useAppStore.getState().showError(message);
      throw error;
    } finally {
      set({ isClassifyingWithAI: false, aiClassificationProgress: null, activeAIJobId: null });
    }
  },
  classifySelectedFileWithAI: async (fileId = get().selectedFileId) => {
    const selectedId = fileId.trim();
    if (!selectedId) {
      const message = "请先选择一个文件。";
      set({ aiClassificationStatus: message });
      useAppStore.getState().showError(message);
      return { scanned: 0, updated: 0, skipped: 0, needsConfirmation: 0 };
    }
    if (get().isClassifyingWithAI) {
      return { scanned: 0, updated: 0, skipped: 0, needsConfirmation: 0 };
    }
    set({ isClassifyingWithAI: true, aiClassificationStatus: "", aiClassificationProgress: null, activeAIJobId: null });
    try {
      const settings = await tauriApi.getAISettings();
      ensureAIReady(settings.enabled, settings.provider, settings.apiKey, settings.apiKeyConfigured);
      const summary = await tauriApi.classifySelectedFilesWithAI([selectedId]);
      await get().refresh(useAppStore.getState().searchQuery);
      await get().loadOrganizeQueue(get().scope);
      const message = aiClassificationSummaryMessage(summary);
      set({ aiClassificationStatus: message });
      useAppStore.getState().showSuccess(message);
      return summary;
    } catch (error) {
      const message = readableAIClassificationError(error);
      set({ aiClassificationStatus: message });
      useAppStore.getState().showError(message);
      throw error;
    } finally {
      set({ isClassifyingWithAI: false, aiClassificationProgress: null, activeAIJobId: null });
    }
  },
  applyAIClassificationProgress: (aiClassificationProgress) => set((state) => {
    if (!state.isClassifyingWithAI) return state;
    if (state.activeAIJobId && state.activeAIJobId !== aiClassificationProgress.jobId) return state;
    return { aiClassificationProgress, activeAIJobId: state.activeAIJobId ?? aiClassificationProgress.jobId };
  }),
  cancelAIClassification: async () => {
    try {
      await tauriApi.cancelAIClassification();
      const message = "AI 分类已取消。";
      set({ aiClassificationStatus: message, isClassifyingWithAI: false, aiClassificationProgress: null, activeAIJobId: null });
      useAppStore.getState().showSuccess(message);
    } catch (error) {
      const message = readableError(error);
      set({ aiClassificationStatus: message, isClassifyingWithAI: false, aiClassificationProgress: null, activeAIJobId: null });
      useAppStore.getState().showError(message);
      throw error;
    }
  },
  clearAIClassificationStatus: () => set({ aiClassificationStatus: "" }),
  refresh: async (query) => {
    const scope = get().scope;
    await Promise.all([get().loadStats(scope), get().loadFirstPage(query, scope, get().libraryFilter)]);
  }
}));

export function getSelectedFile() {
  const { libraryPage, selectedFileId } = useFileLibraryStore.getState();
  return libraryPage.files.find((file) => file.id === selectedFileId) ?? libraryPage.files[0];
}

function ensureAIReady(enabled: boolean, provider: string, apiKey: string, apiKeyConfigured?: boolean) {
  if (!enabled) {
    throw new Error("请先在设置中启用 AI。");
  }
  if (provider !== "ollama" && !apiKey.trim() && !apiKeyConfigured) {
    throw new Error("当前模型服务需要 API Key，请在 AI 设置中填写。");
  }
}

function aiClassificationSummaryMessage(summary: RuleExecutionSummary) {
  const base = `AI 分类完成：扫描 ${summary.scanned.toLocaleString()} 个，更新 ${summary.updated.toLocaleString()} 个，跳过 ${summary.skipped.toLocaleString()} 个，需要确认 ${summary.needsConfirmation.toLocaleString()} 个。`;
  return summary.warning ? `${base}${summary.warning}` : base;
}

export function readableAIClassificationError(error: unknown) {
  const message = readableError(error);
  const normalized = message.toLowerCase();
  if (message.includes("模型返回") || message.includes("Zen Canvas 需要的 JSON")) return message;
  if (message.includes("AI 未启用") || message.includes("启用 AI")) return "请先在设置中启用 AI。";
  if (isRateLimitError(normalized)) {
    return withProviderDetail("模型服务请求过快或达到限流，请减少本次处理数量或稍后重试。", message);
  }
  if (isTimeoutError(normalized)) {
    return withProviderDetail("模型请求超时，请减少本次处理数量、稍后重试，或改用更稳定的模型。", message);
  }
  if (isHttpStatus(normalized, 400)) {
    return withProviderDetail("模型服务拒绝了请求参数，请检查 AI 服务配置后重试。", message);
  }
  if (isHttpStatus(normalized, 401) || isHttpStatus(normalized, 403)) {
    return withProviderDetail("API Key 无效或权限不足，请检查密钥和模型权限。", message);
  }
  if (message.includes("API Key 缺失") || message.includes("当前模型服务需要 API Key")) {
    return "当前模型服务需要 API Key，请在 AI 设置中填写。";
  }
  if (hasConcreteProviderDetail(normalized)) return message;
  if (
    normalized.includes("request failed") ||
    normalized.includes("ollama") ||
    normalized.includes("network")
  ) {
    return "无法连接到模型服务，请检查 Base URL、Chat Path 和网络。";
  }
  if (normalized.includes("invalid json") || normalized.includes("not valid json") || normalized.includes("json")) {
    return "模型没有返回有效结果，请换用更稳定的模型或稍后重试。";
  }
  if (
    normalized.includes("unsupported value") ||
    normalized.includes("targettemplate") ||
    normalized.includes("safety") ||
    message.includes("安全") ||
    message.includes("校验")
  ) {
    return "AI 返回了不安全的路径或操作，Zen Canvas 已拒绝应用该结果。";
  }
  return message;
}

function isHttpStatus(normalized: string, status: number) {
  const text = String(status);
  return normalized.includes(`http ${text}`)
    || normalized.includes(`http status ${text}`)
    || normalized.includes(`status ${text}`)
    || normalized.includes(`status=${text}`);
}

function isRateLimitError(normalized: string) {
  return isHttpStatus(normalized, 429)
    || normalized.includes("rate limit")
    || normalized.includes("too many request");
}

function isTimeoutError(normalized: string) {
  return normalized.includes("timeout") || normalized.includes("timed out");
}

function hasConcreteProviderDetail(normalized: string) {
  return normalized.includes("http ")
    || normalized.includes("http status")
    || normalized.includes("status ")
    || normalized.includes("batch ")
    || normalized.includes("provider response summary")
    || normalized.includes("provider error:")
    || normalized.includes("rate limit");
}

function withProviderDetail(summary: string, detail: string) {
  return detail.includes(summary) ? detail : `${summary}\n${detail}`;
}
