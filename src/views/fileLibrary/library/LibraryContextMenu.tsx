import { useEffect, useRef, type KeyboardEvent } from "react";
import type { FileLibrarySummary } from "../../../types/domain";
import type { Translator } from "../../../types/ui";
import { cn, focusVisibleState } from "../../../utils/tw";
import { libraryRevealLabel } from "../presentation/fileLibraryPresentation";

export interface LibraryContextMenuState {
  file: FileLibrarySummary;
  x: number;
  y: number;
  restoreFocusElement: HTMLElement | null;
}

export interface FileLibraryContextMenuItem {
  label: string;
  action: (trigger: HTMLElement | null) => void;
  disabled?: boolean;
  title?: string;
}

export function libraryTransferMenuItems({
  t,
  windows,
  isDirectory,
  clipboardIncludesDirectory,
  closeContextMenu,
  requestAction,
}: {
  t: Translator;
  windows: boolean;
  isDirectory: boolean;
  clipboardIncludesDirectory: boolean;
  closeContextMenu: (reason?: "action" | "dialog-handoff") => void;
  requestAction: (action: "copy" | "cut" | "paste" | "basket") => void;
}): FileLibraryContextMenuItem[] {
  return (["copy", "cut", "paste", "basket"] as const).map((action) => {
    const disabled = windows && (action === "paste" ? clipboardIncludesDirectory : isDirectory);
    return {
      label: t(action === "basket" ? "fmAddBasket" : action === "copy" ? "fmCopy" : action === "cut" ? "fmCut" : "fmPaste"),
      disabled,
      title: disabled ? t("fmWindowsDirectories") : undefined,
      action: () => {
        closeContextMenu(action === "paste" ? "dialog-handoff" : "action");
        requestAction(action);
      },
    };
  });
}

export function FileLibraryContextMenu({
  x,
  y,
  title,
  ariaLabel,
  items,
  onClose
}: {
  x: number;
  y: number;
  title: string;
  ariaLabel: string;
  items: readonly FileLibraryContextMenuItem[];
  onClose: () => void;
}) {
  const itemRefs = useRef<HTMLButtonElement[]>([]);

  useEffect(() => {
    itemRefs.current.find((item) => item && !item.disabled)?.focus();
  }, []);

  function handleKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    const focusable = itemRefs.current.filter((item): item is HTMLButtonElement => Boolean(item && !item.disabled));
    const activeIndex = focusable.indexOf(document.activeElement as HTMLButtonElement);
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
      return;
    }
    if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
      event.preventDefault();
      if (!focusable.length) return;
      const nextIndex = event.key === "Home"
        ? 0
        : event.key === "End"
          ? focusable.length - 1
          : event.key === "ArrowDown"
            ? (activeIndex + 1 + focusable.length) % focusable.length
            : (activeIndex - 1 + focusable.length) % focusable.length;
      focusable[nextIndex]?.focus();
      return;
    }
    if (event.key === "Tab") {
      event.preventDefault();
      if (focusable.length) {
        focusable[(activeIndex + (event.shiftKey ? -1 : 1) + focusable.length) % focusable.length]?.focus();
      }
      return;
    }
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      if (activeIndex >= 0) {
        const itemIndex = itemRefs.current.indexOf(focusable[activeIndex]);
        items[itemIndex]?.action(focusable[activeIndex] ?? null);
      }
    }
  }

  return (
    <div
      className="fixed z-50 grid max-h-screen min-w-52 gap-1 overflow-y-auto overscroll-contain rounded-[var(--zc-radius-floating)] border border-[var(--zc-border-strong)] bg-[var(--zc-surface-overlay)] p-2 shadow-[var(--zc-shadow-menu)]"
      style={{ left: x, top: y }}
      role="menu"
      aria-label={ariaLabel}
      tabIndex={-1}
      onKeyDown={handleKeyDown}
      onPointerDown={(event) => event.stopPropagation()}
    >
      <p className="truncate px-3 py-1 text-xs font-semibold text-[var(--zc-text-tertiary)]" title={title}>
        {title}
      </p>
      {items.map((item, index) => (
        <button
          key={`${item.label}-${index}`}
          ref={(element) => {
            if (element) itemRefs.current[index] = element;
          }}
          type="button"
          role="menuitem"
          disabled={item.disabled}
          title={item.title}
          className={cn(
            "flex min-h-9 items-center rounded-[var(--zc-radius-control)] px-3 text-left text-sm text-[var(--zc-text-secondary)]",
            "hover:bg-[var(--zc-surface-hover)] hover:text-[var(--zc-text-primary)]",
            "disabled:cursor-not-allowed disabled:opacity-50 disabled:hover:bg-transparent disabled:hover:text-[var(--zc-text-secondary)]",
            focusVisibleState
          )}
          onClick={(event) => { if (!item.disabled) item.action(event.currentTarget); }}
        >
          {item.label}
        </button>
      ))}
    </div>
  );
}

export function LibraryContextMenu({
  context,
  t,
  onClose,
  onPreview,
  onReveal,
  onRename,
  onMove,
  transferItems = [],
  onOpenContent,
  onViewOperations,
  onViewSuggestions,
  onClearSelection
}: {
  context: LibraryContextMenuState;
  t: Translator;
  onClose: () => void;
  onPreview: (trigger: HTMLElement | null) => void;
  onReveal: () => void;
  onRename?: () => void;
  onMove?: () => void;
  transferItems?: FileLibraryContextMenuItem[];
  onOpenContent: () => void;
  onViewOperations: () => void;
  onViewSuggestions: () => void;
  onClearSelection: () => void;
}) {
  const items: Array<{ label: string; action: (trigger: HTMLElement | null) => void }> = [
    ...transferItems,
    { label: t("libraryPreview"), action: onPreview },
    ...(onMove ? [{ label: t("libraryManualMove"), action: () => onMove() }] : []),
    ...(onRename ? [{ label: t("libraryManualRename"), action: () => onRename() }] : []),
    { label: libraryRevealLabel(t), action: () => { onReveal(); onClose(); } },
    { label: t("contentOpen"), action: () => onOpenContent() },
    { label: t("libraryReviewOperations"), action: () => { onViewOperations(); onClose(); } },
    { label: t("libraryViewSuggestions"), action: () => { onViewSuggestions(); onClose(); } },
    { label: t("libraryClearSelection"), action: () => { onClearSelection(); onClose(); } }
  ];

  return (
    <FileLibraryContextMenu
      x={context.x}
      y={context.y}
      title={context.file.name}
      ariaLabel={t("libraryContextMenu")}
      items={items}
      onClose={onClose}
    />
  );
}
