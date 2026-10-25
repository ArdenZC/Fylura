import { useEffect } from "react";
import { tauriApi, type WatcherReconciliationStatus } from "../api/tauriApi";
import { makeTranslator } from "../i18n";
import { useAppStore } from "../store/useAppStore";
import { useWatcherStatusStore } from "../store/useWatcherStatusStore";
import { registerListenerGroup } from "../utils/registerListenerGroup";
import { readableError } from "../utils/viewHelpers";
import { deriveWatcherPresentation } from "../utils/watcherPresentation";
import {
  WatcherRetryQueue,
  WATCHER_QUEUE_BATCH_LIMIT,
  watcherQueueSnapshotFromEvent,
  type FsWatchEvent
} from "./fsWatcherQueue";

interface FsWatcherOptions {
  onRefreshData: () => Promise<void>;
  onError?: (message: string) => void;
  rules?: import("../types/domain").Rule[];
  enabled?: boolean;
}

interface FsWatcherWarningEvent {
  message: string;
  path?: string | null;
  limit?: number | null;
}

function isWatcherStatusSnapshot(payload: FsWatcherWarningEvent | WatcherReconciliationStatus): payload is WatcherReconciliationStatus {
  return Boolean(payload)
    && typeof payload === "object"
    && "scanRootId" in payload
    && typeof payload.scanRootId === "string"
    && "healthStatus" in payload;
}

const WATCHER_FLUSH_DELAY_MS = 500;
export function useFsWatcher({
  onRefreshData,
  onError,
  enabled = true
}: FsWatcherOptions) {
  useEffect(() => {
    if (!enabled) return;

    let disposed = false;
    let statusRefreshTimer: ReturnType<typeof setTimeout> | undefined;
    let legacyCleanup: (() => void | Promise<void>) | undefined;
    let backendCleanup: (() => void | Promise<void>) | undefined;

    const refreshProjection = () => {
      if (statusRefreshTimer !== undefined) clearTimeout(statusRefreshTimer);
      statusRefreshTimer = setTimeout(() => {
        statusRefreshTimer = undefined;
        if (!disposed) void onRefreshData().catch((error) => onError?.(readableError(error)));
      }, 100);
    };

    // Both status channels share the durable store's revision ordering, including
    // snapshots retained across effect restarts and delayed hydration responses.
    const applyStatus = (status: WatcherReconciliationStatus) => {
      const store = useWatcherStatusStore.getState();
      const previous = store.roots[status.scanRootId];
      if (previous && status.rootRevision <= previous.rootRevision) return;
      store.upsert(status);
      refreshProjection();
    };

    const warningHandler = (payload: FsWatcherWarningEvent | WatcherReconciliationStatus) => {
      if (disposed) return;
      if (isWatcherStatusSnapshot(payload)) applyStatus(payload);
      else onError?.(watcherPartialIndexWarningMessage());
    };

    const registerLegacyAdapter = () => {
      if (disposed) return () => {};
      const retryQueue = new WatcherRetryQueue();
      let queue = Promise.resolve();
      let flushTimer: ReturnType<typeof setTimeout> | undefined;
      let adapterDisposed = false;
      let groupCleanup: (() => Promise<void>) | undefined;
      const isDisposed = () => disposed || adapterDisposed;

      const flushQueues = () => {
        if (isDisposed()) return;
        queue = queue
          .then(async () => {
            if (isDisposed()) return;
            const batch = retryQueue.takeReady(Date.now(), WATCHER_QUEUE_BATCH_LIMIT);
            if (!batch.length) return;
            let changed = false;
            const stale = batch.filter((item) => item.action === "stale");
            const upsert = batch.filter((item) => item.action === "upsert");
            const classify = batch.filter((item) => item.action === "classify");

            const reportFailure = (items: typeof batch, error: unknown) => {
              const message = readableError(error);
              for (const item of items) {
                const exhausted = retryQueue.markFailure(item);
                if (!isDisposed()) onError?.(exhausted ? watcherRetryExhaustedMessage() : message);
              }
            };

            if (stale.length > 0 && !isDisposed()) {
              try {
                if (isDisposed()) return;
                changed = (await tauriApi.markFilesStaleByPaths(stale.map((item) => item.path))) > 0 || changed;
                if (!isDisposed()) stale.forEach((item) => retryQueue.markSuccess(item));
              } catch (error) {
                reportFailure(stale, error);
              }
            }
            let upserted = 0;
            if (upsert.length > 0 && !isDisposed()) {
              try {
                if (isDisposed()) return;
                upserted = await tauriApi.upsertFilesByPaths(upsert.map((item) => item.path));
                changed = upserted > 0 || changed;
                if (!isDisposed()) upsert.forEach((item) => retryQueue.markSuccess(item));
                if (upserted > 0 && !isDisposed()) {
                  for (const item of upsert) {
                    const classification = retryQueue.enqueue(item.path, "classify");
                    if (classification) classify.push(classification);
                  }
                }
              } catch (error) {
                reportFailure(upsert, error);
              }
            }
            if (classify.length > 0 && !isDisposed()) {
              try {
                const executeAuthoritativeRulesForPaths = tauriApi.executeAuthoritativeRulesForPaths;
                if (typeof executeAuthoritativeRulesForPaths !== "function") {
                  throw new Error("Authoritative watcher rule classifier is unavailable.");
                }
                await executeAuthoritativeRulesForPaths(classify.map((item) => item.path));
                if (!isDisposed()) classify.forEach((item) => retryQueue.markSuccess(item));
              } catch (error) {
                reportFailure(classify, error);
              }
            }
            if (changed && !isDisposed()) await onRefreshData();
          })
          .catch((error) => {
            if (!isDisposed()) onError?.(readableError(error));
          })
          .finally(() => {
            if (!isDisposed() && retryQueue.hasReadyOrWaiting()) scheduleFlush();
          });
      };

      const scheduleFlush = () => {
        if (isDisposed()) return;
        if (flushTimer !== undefined) clearTimeout(flushTimer);
        const retryDelay = retryQueue.nextRetryDelay();
        flushTimer = setTimeout(() => {
          flushTimer = undefined;
          flushQueues();
        }, retryDelay === 0 ? WATCHER_FLUSH_DELAY_MS : retryDelay ?? WATCHER_FLUSH_DELAY_MS);
      };

      const disposeAdapter = () => {
        if (adapterDisposed) return;
        adapterDisposed = true;
        if (flushTimer !== undefined) clearTimeout(flushTimer);
        flushTimer = undefined;
        retryQueue.clear();
        void groupCleanup?.();
      };

      void registerListenerGroup([
        () => tauriApi.onFsEvent<FsWatchEvent>((payload) => {
          if (!payload || isDisposed()) return;
          const snapshot = watcherQueueSnapshotFromEvent(payload);
          if (!snapshot.stale.length && !snapshot.upsert.length) return;
          snapshot.stale.forEach((path) => retryQueue.enqueue(path, "stale"));
          snapshot.upsert.forEach((path) => retryQueue.enqueue(path, "upsert"));
          scheduleFlush();
        }),
        () => tauriApi.onFsWatcherWarning<FsWatcherWarningEvent>((payload) => warningHandler(payload))
      ]).then((cleanup) => {
        groupCleanup = cleanup;
        if (isDisposed()) void cleanup();
      }).catch((error) => {
        if (!isDisposed()) onError?.(readableError(error));
      });

      return disposeAdapter;
    };

    const registerBackendProjection = () => {
      let projectionDisposed = false;
      let groupCleanup: (() => Promise<void>) | undefined;
      const isDisposed = () => disposed || projectionDisposed;
      const disposeProjection = () => {
        if (projectionDisposed) return;
        projectionDisposed = true;
        void groupCleanup?.();
      };
      if (typeof tauriApi.listScanRoots === "function") {
        void tauriApi.listScanRoots().then((roots) => {
          if (isDisposed()) return;
          useWatcherStatusStore.getState().hydrate(roots);
          const pending = Object.values(useWatcherStatusStore.getState().roots)
            .some((root) => root.needsReconciliation || root.watcherRevision > root.watcherAppliedRevision);
          if (pending) refreshProjection();
        }).catch((error) => {
          if (!isDisposed()) onError?.(readableError(error));
        });
      }
      void registerListenerGroup([
        () => tauriApi.onWatcherReconciliationStatus((payload) => {
          if (!payload || isDisposed()) return;
          applyStatus(payload);
        }),
        () => tauriApi.onFsWatcherWarning<FsWatcherWarningEvent>((payload) => warningHandler(payload))
      ]).then((cleanup) => {
        groupCleanup = cleanup;
        if (isDisposed()) void cleanup();
      }).catch((error) => {
        if (!isDisposed()) onError?.(readableError(error));
      });
      return disposeProjection;
    };

    if (typeof tauriApi.getRuntimeCapabilities !== "function") {
      legacyCleanup = registerLegacyAdapter();
    } else {
      void tauriApi.getRuntimeCapabilities().then((capabilities) => {
        if (disposed) return;
        if (capabilities.backendWatcherReconciliation !== false) backendCleanup = registerBackendProjection();
        else legacyCleanup = registerLegacyAdapter();
      }).catch((error) => {
        if (!disposed) onError?.(readableError(error));
      });
    }

    return () => {
      disposed = true;
      if (statusRefreshTimer !== undefined) clearTimeout(statusRefreshTimer);
      void legacyCleanup?.();
      void backendCleanup?.();
    };
  }, [enabled, onError, onRefreshData]);
}

export function watcherPartialIndexWarningMessage() {
  return makeTranslator(useAppStore.getState().language)(deriveWatcherPresentation({ healthStatus: "partial" }).messageKey);
}

export function watcherRetryExhaustedMessage() {
  return makeTranslator(useAppStore.getState().language)(deriveWatcherPresentation({ healthStatus: "retry_exhausted" }).messageKey);
}

export function watcherReconciliationMessage() {
  return makeTranslator(useAppStore.getState().language)(deriveWatcherPresentation({ healthStatus: "reconciliation_required" }).messageKey);
}

export function watcherPermissionMessage() {
  return makeTranslator(useAppStore.getState().language)(deriveWatcherPresentation({ healthStatus: "permission_required" }).messageKey);
}
