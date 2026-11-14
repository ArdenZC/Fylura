import { useCallback, useEffect, useRef, useState, type MouseEvent } from "react";
import { prefersReducedMotion } from "../../../utils/motion";
import type { FileLibrarySummary } from "../../../types/domain";
import { resolveLibraryContextMenuTarget } from "../list/contextMenuTarget";
import type { LibrarySourceOwner } from "./librarySourceOwner";
import type { LibraryContextMenuState } from "./LibraryContextMenu";

type LibraryContextSource = Pick<
  LibrarySourceOwner,
  "files" | "focusedId" | "selection" | "selectionContainsFileId" | "setExplicitSelection"
>;

type CloseReason = "escape" | "outside-pointer" | "action" | "dialog-handoff";

export function useLibraryContextMenu({
  source,
  restoreFocus
}: {
  source: LibraryContextSource;
  restoreFocus: (target: HTMLElement | null) => void;
}) {
  const [contextMenu, setContextMenu] = useState<LibraryContextMenuState | null>(null);
  const [contextClosing, setContextClosing] = useState(false);
  const closeTimerRef = useRef<number | null>(null);

  const cancelPendingClose = useCallback(() => {
    if (closeTimerRef.current !== null) {
      window.clearTimeout(closeTimerRef.current);
      closeTimerRef.current = null;
    }
    setContextClosing(false);
  }, []);

  useEffect(() => () => {
    if (closeTimerRef.current !== null) window.clearTimeout(closeTimerRef.current);
  }, []);

  const openContextMenu = useCallback((
    file: FileLibrarySummary,
    anchorX?: number,
    anchorY?: number,
    selectTarget = true
  ) => {
    if (selectTarget && !source.selectionContainsFileId(file.id)) {
      source.setExplicitSelection([file.id], file.id, source.files.findIndex((item) => item.id === file.id));
    }
    const row = document.getElementById(`library-row-${file.id}`);
    const rect = row?.getBoundingClientRect();
    const surface = document.querySelector<HTMLElement>(
      '[data-library-source-owner="query-v2"] [data-shared-file-list-source="library"], [data-library-source-owner="query-v2"] [data-shared-file-grid-source="library"]'
    );
    const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const activeInLibrary = Boolean(active && surface?.contains(active) && isValidFocusTarget(active));
    const restoreFocusElement = activeInLibrary
      ? active
      : surface?.isConnected
        ? surface
        : row?.isConnected
          ? row
          : null;
    const width = 260;
    const height = 220;
    cancelPendingClose();
    setContextMenu({
      file,
      restoreFocusElement,
      x: Math.max(8, Math.min(anchorX ?? rect?.left ?? 8, window.innerWidth - width - 8)),
      y: Math.max(8, Math.min(anchorY ?? rect?.bottom ?? 8, window.innerHeight - height - 8))
    });
  }, [cancelPendingClose, source.files, source.selectionContainsFileId, source.setExplicitSelection]);

  const closeContextMenu = useCallback((
    reason: CloseReason = "action",
    pointerTarget: EventTarget | null = null,
    shouldRestoreFocus = reason !== "dialog-handoff"
  ) => {
    if (!contextMenu) return;
    const restoreTarget = contextMenu.restoreFocusElement;
    const finish = () => {
      setContextMenu(null);
      setContextClosing(false);
      if (!shouldRestoreFocus) return;
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
    };
    // Committed actions close in the same frame; only dismissals play the
    // 130ms exit so the menu never delays the operation the user chose.
    if (reason === "action" || reason === "dialog-handoff" || prefersReducedMotion()) {
      cancelPendingClose();
      finish();
      return;
    }
    if (contextClosing) return;
    setContextClosing(true);
    closeTimerRef.current = window.setTimeout(() => {
      closeTimerRef.current = null;
      finish();
    }, 130);
  }, [cancelPendingClose, contextClosing, contextMenu, restoreFocus]);

  useEffect(() => {
    if (!contextMenu) return;
    const closeOnPointer = (event: globalThis.PointerEvent) => closeContextMenu("outside-pointer", event.target);
    document.addEventListener("pointerdown", closeOnPointer);
    return () => {
      document.removeEventListener("pointerdown", closeOnPointer);
    };
  }, [closeContextMenu, contextMenu]);

  const openFocusedContextMenu = useCallback(() => {
    const target = resolveLibraryContextMenuTarget({
      files: source.files,
      focusedId: source.focusedId,
      selection: source.selection
    });
    const file = target ? source.files[target.index] : undefined;
    if (file) openContextMenu(file, undefined, undefined, false);
  }, [openContextMenu, source.files, source.focusedId, source.selection]);

  const handleRowContextMenu = useCallback((event: MouseEvent<HTMLDivElement>, index: number) => {
    event.preventDefault();
    const file = source.files[index];
    if (file) openContextMenu(file, event.clientX, event.clientY);
  }, [openContextMenu, source.files]);

  return { contextMenu, contextClosing, openContextMenu, closeContextMenu, openFocusedContextMenu, handleRowContextMenu };
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
