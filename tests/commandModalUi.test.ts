import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { makeTranslator } from "../src/i18n";

function read(relativePath: string) {
  return readFileSync(resolve(relativePath), "utf8");
}

describe("command modal spotlight polish", () => {
  it("uses shared primitives for empty and failed global-index states", () => {
    const commandModal = read("src/components/CommandModal.tsx");

    expect(commandModal).toContain("StateBlock");
    expect(commandModal).toContain('data-result-kind="global"');
    expect(commandModal).toContain('isSortingPreviewShortcut(event) && activeResult?.kind !== "global"');
    expect(commandModal).toContain("formatDisplayPath");
    expect(commandModal).toContain("compactPath(formatDisplayPath(entry.path");
    expect(commandModal).toContain("globalSearchIndexMeta");
    expect(commandModal).not.toContain("searchScope");
    expect(commandModal).toContain('queryState === "failed" ? "error"');
    expect(commandModal).not.toContain("CommandEmptyState");
  });

  it("uses neutral global-entry icons without AI classification badges", () => {
    const commandModal = read("src/components/CommandModal.tsx");
    const icon = read("src/components/FileTypeIcon.tsx");

    expect(commandModal).not.toContain("FileTypeIcon");
    expect(commandModal).toContain("entry.isDirectory ? <Folder size={20} /> : <FileIcon size={20} />");
    expect(icon).toContain("fileIconForRecord");
    expect(icon).toContain("aria-hidden=\"true\"");
    expect(icon).toContain("if (kind === \"folder\") return Folder");
    expect(icon).toContain("if (kind === \"archive\") return Archive");
    expect(icon).toContain("if (file.file_type === \"Installer\") return Package");
    expect(commandModal).not.toContain("file.purpose");
    expect(commandModal).not.toContain("file.risk_level");
    expect(commandModal).not.toContain("ToneBadge");
  });

  it("keeps spotlight controls accessible and avoids scale-based motion", () => {
    const commandModal = read("src/components/CommandModal.tsx");

    expect(commandModal).toContain('aria-label={t("commandClearSearch")}');
    expect(commandModal).toContain('title={t("commandClearSearch")}');
    expect(commandModal).toContain("aria-live");
    expect(commandModal).toContain('aria-busy={queryState === "pending"}');
    expect(commandModal).toContain('role={standalone ? "search" : "dialog"}');
    expect(commandModal).toContain("aria-modal={standalone ? undefined : true}");
    expect(commandModal).toContain('aria-label={t("globalSearch")}');
    expect(commandModal).toContain("shortcutHints");
    expect(commandModal).not.toContain("scale-");
  });

  it("uses the standalone floating Spotlight alignment and restrained motion contract", () => {
    const commandModal = read("src/components/CommandModal.tsx");

    expect(commandModal).toContain(
      '"w-full overflow-hidden border border-[var(--zc-border-strong)] bg-[var(--zc-surface-overlay)] text-[var(--zc-text-primary)] shadow-[var(--zc-shadow-spotlight)]"'
    );
    expect(commandModal).not.toContain("transition-[border-radius]");
    expect(commandModal).toContain(
      'relative z-10 flex h-full w-full items-start justify-center bg-transparent pt-8 px-8'
    );
    expect(commandModal).toContain(
      'fixed inset-0 z-40 flex items-start justify-center bg-[var(--zc-overlay)] px-5 pt-[15vh] sm:pt-[20vh]'
    );
    expect(commandModal).toContain("<motion.div");
    expect(commandModal).toContain("layout");
    expect(commandModal).toContain('window.addEventListener("blur", handleBlur)');
    expect(commandModal).toContain('window.removeEventListener("blur", handleBlur)');
    expect(commandModal).toContain("const blurredSnapshot = searchWindowSnapshotRef.current;");
    expect(commandModal).toContain("requestSearchWindowHide(blurredSnapshot)");
    expect(commandModal).toContain("initial={prefersReducedMotion ? false : { opacity: 0, y: -8, scale: 0.96 }}");
    expect(commandModal).toContain("animate={{ opacity: 1, y: 0, scale: 1 }}");
    expect(commandModal).toContain("exit={{ opacity: 0, scale: 0.985 }}");
    expect(commandModal).toContain("prefersReducedMotion");
    expect(commandModal).toContain("? { duration: 0 }");
    expect(commandModal).toContain("duration: 0.24");
    expect(commandModal).toContain("[0.34, 1.38, 0.64, 1]");
  });

  it("uses polished shortcut badges and active result text treatment", () => {
    const commandModal = read("src/components/CommandModal.tsx");

    expect(commandModal).toContain("CornerDownLeft");
    expect(commandModal).toContain("font-mono");
    expect(commandModal).not.toContain("font-sans font-medium");
    expect(commandModal).toContain('function ShortcutHint({ badge, label }: { badge: React.ReactNode; label: string })');
    expect(commandModal).toContain('<ShortcutHint badge={<CornerDownLeft className="w-3 h-3" />} label={t("commandOpenHint")} />');
    expect(commandModal).toContain('<ShortcutHint badge="↑↓" label={t("commandNavigateHint")} />');
    expect(commandModal).toContain('<ShortcutHint badge="ESC" label={t("commandCloseHint")} />');
    expect(commandModal).toContain('<strong className={commandFileName}>');
    expect(commandModal).toContain('className="text-[var(--zc-primary)]"');
    expect(commandModal).not.toContain('ShortcutHint badge="↵"');
    expect(commandModal).not.toContain("badge={locateKey}");
    expect(commandModal).not.toContain("badge={sortingPreviewKey}");
    expect(commandModal).not.toContain("to navigate");
    expect(commandModal).not.toContain("to close");
    expect(commandModal).toContain('"bg-transparent font-semibold text-[var(--zc-primary-text)]"');
    expect(commandModal).not.toContain('bg-[var(--zc-primary-soft)] px-1 text-[var(--zc-primary-text)]');
  });

  it("keeps standalone idle spotlight collapsed to the search pill", () => {
    const commandModal = read("src/components/CommandModal.tsx");

    expect(commandModal).toContain("const isStandaloneCollapsed =");
    expect(commandModal).toContain("standalone");
    expect(commandModal).toContain("!trimmedSearch");
    expect(commandModal).toContain('queryState === "idle"');
    expect(commandModal).not.toContain("isScopedEmpty");
    expect(commandModal).toContain("const shouldShowIdleState = !standalone && !trimmedSearch");
    expect(commandModal).toContain("isStandaloneCollapsed ? commandShellCollapsed : commandShellExpanded");
    expect(commandModal).toContain("h-16 w-full max-w-[720px] rounded-full");
    expect(commandModal).toContain("standaloneSearchWindowCollapsedHeight = 160");
    expect(commandModal).toContain("standaloneSearchWindowExpandedHeight = 660");
    expect(commandModal).not.toContain("px-5 pt-2");
    expect(commandModal).not.toContain("pt-[9vh]");
  });

  it("adds product copy for spotlight states and shortcut hints", () => {
    const zh = makeTranslator("zh");
    const en = makeTranslator("en");

    expect(zh("commandIdleTitle")).toBe("输入关键词开始检索");
    expect(zh("commandTypingTitle")).toBe("正在准备搜索");
    expect(zh("commandOpenHint")).toBe("打开结果");
    expect(zh("globalSearch")).toBe("搜索文件夹、文件、操作或设置");
    expect(zh("commandPlaceholder")).toBe("搜索文件、文件夹、操作或设置");
    expect(zh("smartMatches")).toBe("智能匹配");
    expect(zh("unknown")).toBe("未分类或未知类型");
    for (const forbidden of ["Smart Matches", "Unknown", "生命周期维度", "控制指令"]) {
      expect([zh("globalSearch"), zh("commandPlaceholder"), zh("smartMatches"), zh("unknown")].join(" ")).not.toContain(forbidden);
    }
    expect(en("commandIdleTitle")).toBe("Type to search");
    expect(zh("globalSearchIndexMeta")).toContain("全局索引");
    expect(en("globalSearchIndexMeta")).toContain("Global index");
    expect(en("commandClearSearch")).toBe("Clear Spotlight search");
  });

  it("keeps clear, escape, focus, active descendant, selection, and scrolling semantics", () => {
    const commandModal = read("src/components/CommandModal.tsx");

    expect(commandModal).toContain('aria-label={t("commandClearSearch")}');
    expect(commandModal).toContain('title={t("commandClearSearch")}');
    expect(commandModal).toContain("<ModalPortal initialFocusRef={inputRef}");
    expect(commandModal).toContain("initialFocusRef={inputRef}");
    expect(commandModal).toContain("onEscape={closeSpotlight}");
    expect(commandModal).not.toContain("cycleDialogFocus");
    expect(commandModal).toContain("aria-activedescendant={activeResultId}");
    expect(commandModal).toContain("aria-selected={active}");
    expect(commandModal).toContain('scrollIntoView({ block: "nearest" })');
    expect(commandModal).toContain('event.key === "Home"');
    expect(commandModal).toContain('event.key === "End"');
    expect(commandModal).toContain('event.key === "PageDown"');
    expect(commandModal).toContain('event.key === "PageUp"');
    expect(commandModal).toContain('event.key === "Tab"');
    expect(commandModal).toContain("event.nativeEvent.isComposing");
    expect(commandModal).toContain("event.nativeEvent.keyCode === 229");
    expect(commandModal).toContain("onCompositionStart");
    expect(commandModal).toContain("onCompositionEnd");
  });

  it("restores Spotlight setting commands to their target section heading", () => {
    const commandModal = read("src/components/CommandModal.tsx");

    expect(commandModal).toContain("settingsCommandSectionRef");
    expect(commandModal).toContain('requestedSection === "settings-search-scope" ? "settings-search" : requestedSection');
    expect(commandModal).toContain('[data-settings-section-heading]');
    expect(commandModal).toContain("restoreSpotlightFocus");
  });
});
