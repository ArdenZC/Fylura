import { usePaneProjection } from "../../fileLibrary/workbench/PaneContext";
import { ArrowDown, ArrowUp, Bookmark, Pencil, Plus, RefreshCw, Save, Tag, Trash2, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { ModalPortal } from "../../../components/modal/ModalPortal";
import {
  useFileLibrarySavedViewStore,
  useFileLibraryTagStore
} from "../../../store/useFileLibraryV2Store";
import type {
  FileQuerySpecV2,
  LibrarySavedView,
  LibrarySelectionV1,
  UserTag
} from "../../../types/domain";
import type { Translator } from "../../../types/ui";
import { readableError } from "../../../utils/viewHelpers";
import { buttonSecondary, buttonSubtle, cn, buttonPrimary, inputSurface, raisedSurface } from "../../../utils/tw";
import { ConfirmDialog } from "../../shared/ui";

const TAG_COLORS = ["neutral", "blue", "green", "yellow", "red", "purple", "teal", "orange"] as const;

type ManagerKind = "tags" | "saved_views";

export function LibraryMetadataManagerDialog({
  kind,
  query,
  selection,
  selectionCount,
  activeViewId,
  t,
  onApplyView,
  onMutated,
  onClose
}: {
  kind: ManagerKind | null;
  query: FileQuerySpecV2;
  selection: LibrarySelectionV1 | null;
  selectionCount: number | null;
  activeViewId: string | null;
  t: Translator;
  onApplyView: (view: LibrarySavedView) => void;
  onMutated: () => void | Promise<void>;
  onClose: () => void;
}) {
  const closeRef = useRef<HTMLButtonElement | null>(null);
  if (!kind) return null;
  return (
    <ModalPortal initialFocusRef={closeRef} restoreFocus={() => document.querySelector<HTMLElement>(`[data-library-manager="${kind}"]`)} onEscape={onClose}>
      <div className="fixed inset-0 z-50 grid place-items-center overflow-y-auto bg-[var(--zc-overlay)] p-4">
        <section
          className={cn(raisedSurface, "grid max-h-[min(760px,calc(100vh-2rem))] w-full max-w-3xl grid-rows-[auto_minmax(0,1fr)] overflow-hidden p-0")}
          role="dialog"
          aria-modal="true"
          aria-labelledby="library-metadata-manager-title"
        >
          <header className="flex items-start justify-between gap-3 border-b border-[var(--zc-border)] px-5 py-4">
            <div>
              <h2 id="library-metadata-manager-title" className="m-0 text-lg font-semibold text-[var(--zc-text-primary)]">
                {kind === "tags" ? t("libraryManageTagsTitle") : t("libraryManageSavedViews")}
              </h2>
              <p className="mt-1 text-sm text-[var(--zc-text-secondary)]">
                {kind === "tags" ? t("libraryManageTagsDesc") : t("libraryManageSavedViewsDesc")}
              </p>
            </div>
            <button ref={closeRef} className={buttonSubtle} onClick={onClose} aria-label={t("libraryCloseManager")}><X size={16} /></button>
          </header>
          <div className="min-h-0 overflow-y-auto overscroll-contain p-5">
            {kind === "tags"
              ? <TagManager selection={selection} selectionCount={selectionCount} onMutated={onMutated} t={t} />
              : <SavedViewManager query={query} activeViewId={activeViewId} onApplyView={onApplyView} t={t} />}
          </div>
        </section>
      </div>
    </ModalPortal>
  );
}

function TagManager({
  selection,
  selectionCount,
  onMutated,
  t
}: {
  selection: LibrarySelectionV1 | null;
  selectionCount: number | null;
  onMutated: () => void | Promise<void>;
  t: Translator;
}) {
  const { useFileLibraryTagStore } = usePaneProjection();
  const tags = useFileLibraryTagStore((state) => state.tags);
  const isLoading = useFileLibraryTagStore((state) => state.isLoading);
  const storeError = useFileLibraryTagStore((state) => state.error);
  const create = useFileLibraryTagStore((state) => state.create);
  const update = useFileLibraryTagStore((state) => state.update);
  const remove = useFileLibraryTagStore((state) => state.remove);
  const mutate = useFileLibraryTagStore((state) => state.mutate);
  const load = useFileLibraryTagStore((state) => state.load);
  const [newName, setNewName] = useState("");
  const [newColor, setNewColor] = useState<(typeof TAG_COLORS)[number]>("neutral");
  const [editing, setEditing] = useState<UserTag | null>(null);
  const [editName, setEditName] = useState("");
  const [editColor, setEditColor] = useState<(typeof TAG_COLORS)[number]>("neutral");
  const [pendingDelete, setPendingDelete] = useState<UserTag | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (editing && !tags.some((tag) => tag.id === editing.id)) setEditing(null);
  }, [editing, tags]);

  async function run(key: string, action: () => Promise<unknown>) {
    setBusyKey(key);
    setError(null);
    try {
      await action();
    } catch (cause) {
      setError(readableError(cause));
      throw cause;
    } finally {
      setBusyKey(null);
    }
  }

  async function createTag() {
    const displayName = newName.trim();
    if (!displayName) return;
    try {
      await run("create", () => create({ displayName, colorToken: newColor }));
      setNewName("");
    } catch {
      // The inline alert owns the failure.
    }
  }

  function startEdit(tag: UserTag) {
    setEditing(tag);
    setEditName(tag.displayName);
    setEditColor(TAG_COLORS.includes(tag.colorToken as (typeof TAG_COLORS)[number]) ? tag.colorToken as (typeof TAG_COLORS)[number] : "neutral");
    setError(null);
  }

  async function saveEdit() {
    if (!editing || !editName.trim()) return;
    try {
      await run(`edit:${editing.id}`, () => update({
        id: editing.id,
        displayName: editName.trim(),
        colorToken: editColor,
        expectedRevision: editing.revision
      }));
      setEditing(null);
    } catch {
      await load();
    }
  }

  async function changeAssignment(tag: UserTag, operation: "add" | "remove") {
    if (!selection) return;
    try {
      await run(`${operation}:${tag.id}`, () => mutate({
        selection,
        tagIds: [tag.id],
        operation,
        expectedCount: selectionCount
      }));
      await onMutated();
    } catch {
      // The inline alert owns the failure.
    }
  }

  async function confirmDelete() {
    if (!pendingDelete) return;
    try {
      await run(`delete:${pendingDelete.id}`, () => remove({
        id: pendingDelete.id,
        confirm: true,
        expectedUsageCount: pendingDelete.usageCount,
        expectedRevision: pendingDelete.revision
      }));
      setPendingDelete(null);
    } catch {
      await load();
    }
  }

  const disabled = busyKey !== null;
  return (
    <div className="grid gap-4">
      <form className="flex flex-wrap items-end gap-2" onSubmit={(event) => { event.preventDefault(); void createTag(); }}>
        <label className="grid min-w-52 flex-1 gap-1 text-sm text-[var(--zc-text-secondary)]">
          {t("libraryTagName")}
          <input className={cn(inputSurface, "min-h-9 px-3")} value={newName} maxLength={64} onChange={(event) => setNewName(event.target.value)} required />
        </label>
        <ColorSelect value={newColor} onChange={setNewColor} disabled={disabled} label={t("libraryTagNewColor")} t={t} />
        <button className={buttonPrimary} type="submit" disabled={disabled || !newName.trim()}><Plus size={15} />{t("libraryTagCreate")}</button>
      </form>
      <p className="text-xs text-[var(--zc-text-secondary)]" aria-live="polite">
        {selection
          ? selectionCount === null
            ? t("libraryTagCountPending")
            : replaceCopy(t("libraryTagSelectionCount"), { count: selectionCount.toLocaleString() })
          : t("libraryTagSelectionHint")}
      </p>
      {(error || storeError) ? <p className="rounded-lg bg-[var(--zc-danger-soft)] px-3 py-2 text-sm text-[var(--zc-danger-text)]" role="alert">{error ?? storeError}</p> : null}
      <div className="grid gap-2" aria-busy={isLoading || disabled}>
        {tags.map((tag) => (
          <div key={tag.id} className="grid gap-2 rounded-xl border border-[var(--zc-border)] bg-[var(--zc-surface-subtle)] p-3 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center">
            {editing?.id === tag.id ? (
              <div className="flex min-w-0 flex-wrap gap-2">
                <input className={cn(inputSurface, "min-h-9 min-w-40 flex-1 px-3")} value={editName} maxLength={64} onChange={(event) => setEditName(event.target.value)} autoFocus aria-label={replaceCopy(t("libraryTagRename"), { name: tag.displayName })} />
                <ColorSelect value={editColor} onChange={setEditColor} disabled={disabled} label={replaceCopy(t("libraryTagColorFor"), { name: tag.displayName })} t={t} />
              </div>
            ) : (
              <div className="min-w-0">
                <strong className="flex items-center gap-2 truncate text-sm text-[var(--zc-text-primary)]"><Tag size={14} aria-hidden="true" />{tag.displayName}</strong>
                <span className="text-xs text-[var(--zc-text-secondary)]">{replaceCopy(t("libraryTagUsage"), { count: tag.usageCount.toLocaleString() })}</span>
              </div>
            )}
            <div className="flex flex-wrap justify-end gap-1">
              {editing?.id === tag.id ? (
                <>
                  <button className={buttonSubtle} onClick={() => void saveEdit()} disabled={disabled || !editName.trim()}><Save size={14} />{t("save")}</button>
                  <button className={buttonSubtle} onClick={() => setEditing(null)} disabled={disabled}>{t("cancel")}</button>
                </>
              ) : (
                <>
                  <button className={buttonSubtle} onClick={() => void changeAssignment(tag, "add")} disabled={!selection || disabled}>{t("libraryTagAssign")}</button>
                  <button className={buttonSubtle} onClick={() => void changeAssignment(tag, "remove")} disabled={!selection || disabled}>{t("libraryTagRemove")}</button>
                  <button className={buttonSubtle} onClick={() => startEdit(tag)} disabled={disabled} aria-label={replaceCopy(t("libraryTagEdit"), { name: tag.displayName })}><Pencil size={14} /></button>
                  <button className={buttonSubtle} onClick={() => setPendingDelete(tag)} disabled={disabled} aria-label={replaceCopy(t("libraryTagDelete"), { name: tag.displayName })}><Trash2 size={14} /></button>
                </>
              )}
            </div>
          </div>
        ))}
        {!isLoading && !tags.length ? <p className="text-sm text-[var(--zc-text-secondary)]">{t("libraryTagEmpty")}</p> : null}
      </div>
      <ConfirmDialog
        open={Boolean(pendingDelete)}
        tone="danger"
        title={t("libraryTagDeleteTitle")}
        description={t("libraryTagDeleteDesc")}
        emphasis={pendingDelete ? `${pendingDelete.displayName} · ${pendingDelete.usageCount.toLocaleString()} uses` : undefined}
        confirmLabel={t("libraryTagDeleteConfirm")}
        cancelLabel={t("cancel")}
        isProcessing={busyKey?.startsWith("delete:") ?? false}
        errorMessage={error ?? undefined}
        onConfirm={confirmDelete}
        onCancel={() => { if (!busyKey) setPendingDelete(null); }}
      />
    </div>
  );
}

function SavedViewManager({
  query,
  activeViewId,
  onApplyView,
  t
}: {
  query: FileQuerySpecV2;
  activeViewId: string | null;
  onApplyView: (view: LibrarySavedView) => void;
  t: Translator;
}) {
  const { useFileLibrarySavedViewStore } = usePaneProjection();
  const views = useFileLibrarySavedViewStore((state) => state.views);
  const isLoading = useFileLibrarySavedViewStore((state) => state.isLoading);
  const storeError = useFileLibrarySavedViewStore((state) => state.error);
  const create = useFileLibrarySavedViewStore((state) => state.create);
  const update = useFileLibrarySavedViewStore((state) => state.update);
  const remove = useFileLibrarySavedViewStore((state) => state.remove);
  const load = useFileLibrarySavedViewStore((state) => state.load);
  const [newName, setNewName] = useState("");
  const [editing, setEditing] = useState<LibrarySavedView | null>(null);
  const [editName, setEditName] = useState("");
  const [pendingDelete, setPendingDelete] = useState<LibrarySavedView | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function run(key: string, action: () => Promise<unknown>) {
    setBusyKey(key);
    setError(null);
    try {
      await action();
    } catch (cause) {
      setError(readableError(cause));
      throw cause;
    } finally {
      setBusyKey(null);
    }
  }

  async function createView() {
    if (!newName.trim()) return;
    try {
      await run("create", () => create({ displayName: newName.trim(), query, position: views.length }));
      setNewName("");
    } catch {
      // The inline alert owns the failure.
    }
  }

  async function writeView(view: LibrarySavedView, patch: Partial<Pick<LibrarySavedView, "displayName" | "query" | "position">>) {
    try {
      await run(`update:${view.id}`, () => update({
        id: view.id,
        displayName: patch.displayName ?? view.displayName,
        query: patch.query ?? view.query,
        position: patch.position ?? view.position,
        expectedRevision: view.revision
      }));
    } catch {
      await load();
      throw new Error("saved_view_revision_conflict");
    }
  }

  async function reorder(view: LibrarySavedView, delta: -1 | 1) {
    const index = views.findIndex((item) => item.id === view.id);
    const other = views[index + delta];
    if (!other) return;
    try {
      await writeView(view, { position: other.position });
      const refreshed = useFileLibrarySavedViewStore.getState().views.find((item) => item.id === other.id) ?? other;
      await writeView(refreshed, { position: view.position });
    } catch {
      // Reload performed by writeView.
    }
  }

  async function saveRename() {
    if (!editing || !editName.trim()) return;
    try {
      await writeView(editing, { displayName: editName.trim() });
      setEditing(null);
    } catch {
      // Reload performed by writeView.
    }
  }

  async function confirmDelete() {
    if (!pendingDelete) return;
    try {
      await run(`delete:${pendingDelete.id}`, () => remove({ id: pendingDelete.id, expectedRevision: pendingDelete.revision }));
      setPendingDelete(null);
    } catch {
      await load();
    }
  }

  const disabled = busyKey !== null;
  return (
    <div className="grid gap-4">
      <form className="flex flex-wrap items-end gap-2" onSubmit={(event) => { event.preventDefault(); void createView(); }}>
        <label className="grid min-w-52 flex-1 gap-1 text-sm text-[var(--zc-text-secondary)]">
          {t("librarySavedViewName")}
          <input className={cn(inputSurface, "min-h-9 px-3")} value={newName} maxLength={128} onChange={(event) => setNewName(event.target.value)} required />
        </label>
        <button className={buttonPrimary} type="submit" disabled={disabled || !newName.trim()}><Plus size={15} />{t("librarySavedViewSave")}</button>
      </form>
      {(error || storeError) ? <p className="rounded-lg bg-[var(--zc-danger-soft)] px-3 py-2 text-sm text-[var(--zc-danger-text)]" role="alert">{error ?? storeError}</p> : null}
      <div className="grid gap-2" aria-busy={isLoading || disabled}>
        {views.map((view, index) => (
          <div key={view.id} className="grid gap-2 rounded-xl border border-[var(--zc-border)] bg-[var(--zc-surface-subtle)] p-3 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center">
            {editing?.id === view.id ? (
                <input className={cn(inputSurface, "min-h-9 px-3")} value={editName} maxLength={128} onChange={(event) => setEditName(event.target.value)} autoFocus aria-label={replaceCopy(t("librarySavedViewRename"), { name: view.displayName })} />
            ) : (
              <div className="min-w-0">
                <strong className="flex items-center gap-2 truncate text-sm text-[var(--zc-text-primary)]"><Bookmark size={14} />{view.displayName}{activeViewId === view.id ? t("librarySavedViewActive") : ""}</strong>
                <span className="text-xs text-[var(--zc-text-secondary)]">{replaceCopy(t("librarySavedViewPosition"), { position: view.position })}</span>
                {view.invalidReferences.length ? <p className="mt-1 text-xs text-[var(--zc-warning-text)]" role="status">{replaceCopy(t("librarySavedViewInvalidReferences"), { references: view.invalidReferences.join(", ") })}</p> : null}
              </div>
            )}
            <div className="flex flex-wrap justify-end gap-1">
              {editing?.id === view.id ? (
                <>
                  <button className={buttonSubtle} onClick={() => void saveRename()} disabled={disabled || !editName.trim()}><Save size={14} />{t("save")}</button>
                  <button className={buttonSubtle} onClick={() => setEditing(null)} disabled={disabled}>{t("cancel")}</button>
                </>
              ) : (
                <>
                  <button className={buttonSubtle} onClick={() => onApplyView(view)} disabled={disabled}>{t("librarySavedViewOpen")}</button>
                  <button className={buttonSubtle} onClick={() => void writeView(view, { query }).catch(() => undefined)} disabled={disabled}><RefreshCw size={14} />{t("librarySavedViewUpdate")}</button>
                  <button className={buttonSubtle} onClick={() => { setEditing(view); setEditName(view.displayName); setError(null); }} disabled={disabled} aria-label={replaceCopy(t("librarySavedViewRename"), { name: view.displayName })}><Pencil size={14} /></button>
                  <button className={buttonSubtle} onClick={() => void reorder(view, -1)} disabled={disabled || index === 0} aria-label={replaceCopy(t("librarySavedViewMoveUp"), { name: view.displayName })}><ArrowUp size={14} /></button>
                  <button className={buttonSubtle} onClick={() => void reorder(view, 1)} disabled={disabled || index === views.length - 1} aria-label={replaceCopy(t("librarySavedViewMoveDown"), { name: view.displayName })}><ArrowDown size={14} /></button>
                  <button className={buttonSubtle} onClick={() => setPendingDelete(view)} disabled={disabled} aria-label={replaceCopy(t("librarySavedViewDelete"), { name: view.displayName })}><Trash2 size={14} /></button>
                </>
              )}
            </div>
          </div>
        ))}
        {!isLoading && !views.length ? <p className="text-sm text-[var(--zc-text-secondary)]">{t("librarySavedViewEmpty")}</p> : null}
      </div>
      <ConfirmDialog
        open={Boolean(pendingDelete)}
        tone="danger"
        title={t("librarySavedViewDeleteTitle")}
        description={t("librarySavedViewDeleteDesc")}
        emphasis={pendingDelete?.displayName}
        confirmLabel={t("librarySavedViewDeleteConfirm")}
        cancelLabel={t("cancel")}
        isProcessing={busyKey?.startsWith("delete:") ?? false}
        errorMessage={error ?? undefined}
        onConfirm={confirmDelete}
        onCancel={() => { if (!busyKey) setPendingDelete(null); }}
      />
    </div>
  );
}

function ColorSelect({
  value,
  onChange,
  disabled,
  label,
  t
}: {
  value: (typeof TAG_COLORS)[number];
  onChange: (value: (typeof TAG_COLORS)[number]) => void;
  disabled: boolean;
  label: string;
  t: Translator;
}) {
  return (
    <label className="grid gap-1 text-sm text-[var(--zc-text-secondary)]">
      {t("libraryTagColor")}
      <select className={cn(inputSurface, "min-h-9 px-2")} value={value} onChange={(event) => onChange(event.target.value as (typeof TAG_COLORS)[number])} disabled={disabled} aria-label={label}>
        {TAG_COLORS.map((color) => <option key={color} value={color}>{color}</option>)}
      </select>
    </label>
  );
}

function replaceCopy(template: string, values: Record<string, string | number>): string {
  return Object.entries(values).reduce((copy, [key, value]) => copy.replaceAll(`{${key}}`, String(value)), template);
}
