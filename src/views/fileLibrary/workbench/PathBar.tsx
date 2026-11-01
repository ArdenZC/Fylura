import {
  ArrowLeft,
  ArrowRight,
  Check,
  Pencil,
  FolderOpen,
  X,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import {
  useChromeContext,
  useI18nContext,
} from "../../../contexts/AppContexts";
import { fileManagementApi } from "../../../api/fileManagementApi";
import { useFileLibraryExperience } from "../FileLibraryExperienceProvider";
import { usePane } from "./PaneContext";
import { readableError } from "../../../utils/viewHelpers";
import { workbenchTargetKey } from "./workbenchModel";

export function pathCrumbs(path: string) {
  const normalized = path.replaceAll("\\", "/").replace(/\/+$/, "");
  const parts = normalized.split("/").filter(Boolean);
  const prefix = normalized.startsWith("//")
    ? "//"
    : normalized.startsWith("/")
      ? "/"
      : "";
  return parts.map((label, index) => ({
    label,
    path:
      prefix +
      parts.slice(0, index + 1).join("/") +
      (index === 0 && /^[A-Za-z]:$/.test(label) ? "/" : ""),
  }));
}
export function PathBar() {
  const pane = usePane();
  const { controller, state } = useFileLibraryExperience();
  const { platform } = useChromeContext();
  const { t } = useI18nContext();
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const input = useRef<HTMLInputElement | null>(null);
  const visibleRef = useRef(pane?.visible ?? true);
  visibleRef.current = pane?.visible ?? true;
  useEffect(() => {
    visibleRef.current = pane?.visible ?? true;
    return () => { visibleRef.current = false; };
  }, []);
  const updateLocation = pane?.updateLocation;
  const target = state.workspace.session.currentTarget;
  const targetKey = workbenchTargetKey(target);
  const sessionId =
    target?.kind === "browse" && target.location.kind === "ephemeral"
      ? target.location.browseSessionId
      : undefined;
  const pathId = target?.kind === "browse" ? target.pathRef.id : undefined;
  useEffect(() => {
    let cancelled = false;
    updateLocation?.("", targetKey);
    setError("");
    if (sessionId && target?.kind === "browse")
      void fileManagementApi
        .fileLibraryPlaces({ sessionId, pathRef: target.pathRef })
        .then((places) => {
          if (!cancelled)
            updateLocation?.(
              places.find((place) => place.current)?.routingHint ?? "",
              targetKey,
            );
        })
        .catch((cause) => {
          if (!cancelled) setError(readableError(cause));
        });
    return () => {
      cancelled = true;
    };
  }, [sessionId, pathId, targetKey, updateLocation]);
  useEffect(() => {
    if (editing && !busy) {
      input.current?.focus();
      input.current?.select();
    }
  }, [editing, busy]);
  useEffect(() => {
    if (!pane?.focused || !pane.visible) return;
    const shortcut = (event: globalThis.KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        event.isComposing ||
        document.querySelector('[aria-modal="true"]')
      )
        return;
      if (
        event.altKey &&
        !event.ctrlKey &&
        !event.metaKey &&
        (event.key === "ArrowLeft" || event.key === "ArrowRight")
      ) {
        event.preventDefault();
        void navigateHistory(event.key === "ArrowLeft" ? "back" : "forward");
        return;
      }
      if (
        event.altKey ||
        !(event.ctrlKey || event.metaKey) ||
        event.key.toLowerCase() !== "l"
      )
        return;
      event.preventDefault();
      setValue(pane.routingHint);
      setEditing(true);
      requestAnimationFrame(() => {
        input.current?.focus();
        input.current?.select();
      });
    };
    window.addEventListener("keydown", shortcut);
    return () => window.removeEventListener("keydown", shortcut);
  }, [pane?.focused, pane?.visible, pane?.routingHint]);
  async function navigate(path: string) {
    if (busy || !path.trim()) return;
    setBusy(true);
    setError("");
    try {
      if (platform !== "win32" && platform !== "darwin")
        throw Error(t("wbPlatformUnavailable"));
      await controller.workspace.releaseReadmittableHistorySessions();
      const admitted = await controller.openBrowse(
        {
          platform: platform === "win32" ? "windows" : "macos",
          routingHint: path.trim(),
          displayHint: path.trim(),
        },
        () => visibleRef.current,
      );
      if (!admitted) throw Error(t("wbNavigationFailed"));
      await controller.workspace.releaseReadmittableHistorySessions();
      setEditing(false);
    } catch (cause) {
      setError(readableError(cause));
    } finally {
      setBusy(false);
    }
  }
  async function navigateHistory(direction: "back" | "forward") {
    if (busy) return;
    setError("");
    setBusy(true);
    try {
      await controller[direction]();
      await controller.workspace.releaseReadmittableHistorySessions();
    } catch (cause) {
      setError(readableError(cause));
    } finally {
      setBusy(false);
    }
  }
  const edit = () => {
    setValue(pane?.routingHint ?? "");
    setEditing(true);
  };
  return (
    <div className="wb-path-section">
      <div className="wb-pathbar" aria-label={t("wbPathNavigation")}>
        <button
          type="button"
          className="wb-icon"
          title={t("fileLibraryBack")}
          aria-label={t("fileLibraryBack")}
          disabled={busy || state.workspace.session.historyIndex <= 0}
          onClick={() => void navigateHistory("back")}
        >
          <ArrowLeft size={15} />
        </button>
        <button
          type="button"
          className="wb-icon"
          title={t("fileLibraryForward")}
          aria-label={t("fileLibraryForward")}
          disabled={
            busy ||
            state.workspace.session.historyIndex >=
              state.workspace.session.history.length - 1
          }
          onClick={() => void navigateHistory("forward")}
        >
          <ArrowRight size={15} />
        </button>
        {pane && <div id={`wb-crumbs-${pane.id}`} className="wb-crumbs" hidden={editing || state.mode !== "browse"} />}
        {editing ? (
          <form
            className="wb-path-edit"
            onSubmit={(event) => {
              event.preventDefault();
              void navigate(value);
            }}
          >
            <input
              ref={input}
              aria-label={t("wbPathInput")}
              value={value}
              disabled={busy}
              placeholder={t("fmPathHint")}
              onChange={(event) => setValue(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Escape") {
                  event.preventDefault();
                  setEditing(false);
                }
              }}
            />
            <button
              className="wb-icon"
              aria-label={t("fmGo")}
              disabled={busy || !value.trim()}
            >
              <Check size={15} />
            </button>
            <button
              type="button"
              className="wb-icon"
              aria-label={t("fmCancel")}
              disabled={busy}
              onClick={() => setEditing(false)}
            >
              <X size={15} />
            </button>
          </form>
        ) : (
          <>
            <nav className="wb-crumbs" hidden={!!pane && state.mode === "browse"} aria-label={t("browseBreadcrumbLabel")}>
              <FolderOpen size={14} />
              {pane?.routingHint ? (
                pathCrumbs(pane.routingHint).map((crumb, index, crumbs) => (
                  <button
                    key={crumb.path}
                    title={crumb.path}
                    aria-current={
                      index === crumbs.length - 1 ? "page" : undefined
                    }
                    onClick={() =>
                      index === crumbs.length - 1
                        ? edit()
                        : void navigate(crumb.path)
                    }
                  >
                    {crumb.label}
                  </button>
                ))
              ) : (
                <button onClick={edit}>
                  {t(
                    state.mode === "library"
                      ? "fileLibraryModeLibrary"
                      : "wbEnterPath",
                  )}
                </button>
              )}
            </nav>
            <button
              className="wb-icon"
              aria-label={t("wbEditPath")}
              title={t("wbEditPath")}
              onClick={edit}
            >
              <Pencil size={14} />
            </button>
          </>
        )}
      </div>
      {error && (
        <p className="wb-path-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
