import { useCallback, useEffect, useState, type MouseEvent } from "react";
import type { BrowsePresentationEntry } from "../presentation/contracts";
import { resolveBrowseContextMenuTarget } from "../list/contextMenuTarget";
import type { BrowseSourceOwner } from "./browseSourceOwner";

export interface BrowseContextMenuState {
  entry: BrowsePresentationEntry;
  x: number;
  y: number;
  restoreFocusElement: HTMLElement | null;
}

type BrowseContextSource = Pick<
  BrowseSourceOwner,
  "entries" | "focusedId" | "selectedIds" | "selectEntry"
>;

type CloseReason = "escape" | "outside-pointer" | "action" | "source-change" | "dialog-handoff";

export function useBrowseContextMenu({
  source,
  restoreFocus
}: {
  source: BrowseContextSource;
  restoreFocus: (target: HTMLElement | null) => void;
}) {
  const [contextMenu, setContextMenu] = useState<BrowseContextMenuState | null>(null);

  const openContextMenu = useCallback((
    entry: BrowsePresentationEntry,
    anchorX?: number,
    anchorY?: number,
    selectTarget = true
  ) => {
    const entryId = entry.entryRef.entryId;
    if (selectTarget && !source.selectedIds.has(entryId)) source.selectEntry(entryId, "replace");
    const row = Array.from(document.querySelectorAll<HTMLElement>("[data-browse-entry-id], [data-browse-grid-entry-id]"))
      .find((candidate) => candidate.dataset.browseEntryId === entryId || candidate.dataset.browseGridEntryId === entryId);
    const listOrGrid = document.querySelector<HTMLElement>(
      "[data-shared-file-list-source=\"browse\"], [data-shared-file-grid-source=\"browse\"]"
    );
    const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const activeInBrowse = Boolean(active && listOrGrid?.contains(active) && isValidFocusTarget(active));
    const restoreFocusElement = activeInBrowse
      ? active
      : listOrGrid?.isConnected
        ? listOrGrid
        : row?.isConnected
          ? row
          : null;
    const rect = row?.getBoundingClientRect();
    const width = 260;
    const height = Math.min(
      window.innerHeight - 16,
      entry.entryKind === "directory" ? 520 : 560,
    );
    setContextMenu({
      entry,
      restoreFocusElement,
      x: clamp(anchorX ?? rect?.left ?? 8, width, window.innerWidth),
      y: clamp(anchorY ?? rect?.bottom ?? 8, height, window.innerHeight)
    });
  }, [source.selectEntry, source.selectedIds]);

  const closeContextMenu = useCallback((
    reason: CloseReason = "action",
    pointerTarget: EventTarget | null = null
  ) => {
    const restoreTarget = contextMenu?.restoreFocusElement ?? null;
    setContextMenu(null);
    if (reason === "source-change" || reason === "dialog-handoff") return;
    requestAnimationFrame(() => {
      if (reason === "outside-pointer") {
        const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
        if (isValidFocusTarget(active)) return;
        const pointerElement = focusablePointerTarget(pointerTarget);
        if (pointerElement) {
          pointerElement.focus();
          if (document.activeElement === pointerElement) return;
        }
      }
      restoreFocus(restoreTarget);
    });
  }, [contextMenu, restoreFocus]);

  useEffect(() => {
    if (!contextMenu) return;
    const closeOnPointer = (event: globalThis.PointerEvent) => closeContextMenu("outside-pointer", event.target);
    document.addEventListener("pointerdown", closeOnPointer);
    return () => {
      document.removeEventListener("pointerdown", closeOnPointer);
    };
  }, [closeContextMenu, contextMenu]);

  const openFocusedContextMenu = useCallback((entryOrEvent?: BrowsePresentationEntry) => {
    // A column can name the row it wants the menu for; the list/grid path
    // resolves it from the source's focus instead. A React onClick handler can
    // also be wired directly, so anything that is not an entry falls back to
    // the focused row rather than being treated as one.
    const entry = entryOrEvent !== null
      && typeof entryOrEvent === "object"
      && "entryRef" in entryOrEvent
      ? entryOrEvent
      : undefined;
    if (entry !== undefined) {
      openContextMenu(entry, undefined, undefined, false);
      return;
    }
    const target = resolveBrowseContextMenuTarget({
      entries: source.entries,
      focusedId: source.focusedId,
      selectedIds: source.selectedIds
    });
    if (target) openContextMenu(target.entry, undefined, undefined, false);
  }, [openContextMenu, source.entries, source.focusedId, source.selectedIds]);

  const handleRowContextMenu = useCallback((event: MouseEvent<HTMLElement>, entry: BrowsePresentationEntry) => {
    event.preventDefault();
    openContextMenu(entry, event.clientX, event.clientY);
  }, [openContextMenu]);

  return { contextMenu, openContextMenu, closeContextMenu, openFocusedContextMenu, handleRowContextMenu };
}

function clamp(value: number, size: number, extent: number) {
  return Math.max(8, Math.min(value, extent - size - 8));
}

function isValidFocusTarget(target: HTMLElement | null) {
  return Boolean(target?.isConnected
    && target !== document.body
    && target !== document.documentElement
    && (target.tabIndex >= 0 || target.matches("button, input, select, textarea, a[href], [contenteditable='true']")));
}

function focusablePointerTarget(target: EventTarget | null) {
  const element = target instanceof HTMLElement ? target : null;
  if (isValidFocusTarget(element)) return element;
  const closest = element?.closest<HTMLElement>("button, input, select, textarea, a[href], [contenteditable='true']") ?? null;
  return isValidFocusTarget(closest) ? closest : null;
}
