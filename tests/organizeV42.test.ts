import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { makeTranslator } from "../src/i18n";
import { operationConfirmationTone, resolveExecutableSelectedPreviews, resolvePreviewEligibility } from "../src/store/useOperationQueueStore";
import type { OperationPreview } from "../src/types/domain";
import { formatPreviewDisplayPath, groupOperationPreviews } from "../src/utils/viewHelpers";
import { PreviewFileRow } from "../src/views/timeline/PreviewFileRow";

const read = (file: string) => readFileSync(resolve(file), "utf8");

function preview(id: string, overrides: Partial<OperationPreview> = {}): OperationPreview {
  return {
    id,
    fileId: `file-${id}`,
    operation_type: "move",
    source_path: `C:/Users/Zen/Downloads/${id}.txt`,
    target_path: `C:/Users/Zen/ZenCanvas/20_Areas/Work/${id}.txt`,
    old_name: `${id}.txt`,
    new_name: `${id}.txt`,
    status: "pending",
    risk_level: "Normal",
    confidence: 0.9,
    requires_confirmation: false,
    selected_by_default: true,
    is_executable: true,
    editable_new_name: true,
    ...overrides,
    reason: overrides.reason ?? "test"
  };
}

describe("Organize Suggestions v4.2 hardening", () => {
  it("keeps button and dialog counts bound to the same executable selection used by the store", () => {
    const timeline = read("src/views/timeline/TimelineView.tsx");
    const store = [
      read("src/store/useOperationQueueStore.ts"),
      read("src/store/operationQueue/selectors.ts"),
      read("src/store/operationQueue/operationExecutionController.ts")
    ].join("\n");
    const good = preview("good");
    const invalid = preview("invalid", { new_name: "bad?.txt" });
    const resolved = resolveExecutableSelectedPreviews([good, invalid], new Set([good.id, invalid.id]), null);

    expect(resolved.selectedCount).toBe(2);
    expect(resolved.operations).toEqual([good]);
    expect(timeline).toContain("const executableSelectedCount = selectedOperations.length");
    expect(timeline).toContain("resolveExecutableSelectedPreviews(displayPreviews, selectedIds, executionIntent)");
    expect(timeline).toContain('disabled={!executableSelectedCount || isExecuting || Boolean(mutationUnavailable)}');
    expect(timeline).toContain('replace("{count}", selectedOperations.length.toLocaleString())');
    expect(store).toContain("resolveExecutableSelectedPreviews(");
    expect(store).toContain("selectedOperationIds");
  });

  it("changes executable count immediately when a selected file name becomes invalid and valid again", () => {
    const selectedIds = new Set(["rename"]);
    expect(resolveExecutableSelectedPreviews([preview("rename")], selectedIds, null).operations).toHaveLength(1);
    expect(resolveExecutableSelectedPreviews([preview("rename", { new_name: "bad?.txt" })], selectedIds, null).operations).toHaveLength(0);
    expect(resolveExecutableSelectedPreviews([preview("rename", { new_name: "restored.txt" })], selectedIds, null).operations).toHaveLength(1);
  });

  it("renders invalid names as a distinct non-success execution state", () => {
    const markup = renderToStaticMarkup(createElement(PreviewFileRow, {
      preview: preview("invalid-row", { new_name: "bad?.txt" }),
      isSelected: true,
      toggle: () => undefined,
      onRenamePreview: () => undefined,
      t: makeTranslator("zh")
    }));
    expect(markup).toContain('data-preview-execution-state="invalid-name"');
    expect(markup).toContain("文件名无效");
    expect(markup).not.toContain('data-preview-execution-state="executable"');
    expect(markup).toContain('aria-invalid="true"');
  });

  it("excludes blocked, backend-inexecutable, stale, and non-whitelisted selections", () => {
    const allowed = preview("allowed");
    const blocked = preview("blocked", { is_executable: false, blocking_reason: "source changed" });
    const injected = preview("injected");
    const intent = { source: "organize" as const, scopeKey: "all", allowedPreviewIds: new Set([allowed.id, blocked.id]), initialAllowedCount: 2, sessionId: "v42" };
    const resolved = resolveExecutableSelectedPreviews([allowed, blocked, injected], new Set([allowed.id, blocked.id, injected.id, "stale"]), intent);
    expect(resolved.operations).toEqual([allowed]);
    expect(resolved).toMatchObject({ blockedCount: 1, outsideWhitelistCount: 1, unavailableCount: 1 });
  });

  it("assigns one prioritized exclusion reason to every non-executable selection", () => {
    const invalid = preview("invalid-reason", { new_name: "bad?.txt" });
    const blocked = preview("blocked-reason", { is_executable: false, blocking_reason: "protected" });
    const unavailable = preview("unavailable-reason", { status: "failed", is_executable: false, new_name: "bad?.txt" });
    const outside = preview("outside-reason", { status: "failed", is_executable: false, new_name: "bad?.txt" });
    const intent = { source: "organize" as const, scopeKey: "all", allowedPreviewIds: new Set([invalid.id, blocked.id, unavailable.id]), initialAllowedCount: 3, sessionId: "exclusive" };

    expect(resolvePreviewEligibility(invalid, intent).reason).toBe("invalidName");
    expect(resolvePreviewEligibility(blocked, intent).reason).toBe("blocked");
    expect(resolvePreviewEligibility(unavailable, intent).reason).toBe("unavailable");
    expect(resolvePreviewEligibility(outside, intent).reason).toBe("outsideWhitelist");

    const resolved = resolveExecutableSelectedPreviews([invalid, blocked, unavailable, outside], new Set([invalid.id, blocked.id, unavailable.id, outside.id]), intent);
    expect(resolved.operations).toEqual([]);
    expect(resolved).toMatchObject({ selectedCount: 4, invalidNameCount: 1, blockedCount: 1, unavailableCount: 1, outsideWhitelistCount: 1, excludedCount: 4 });
  });

  it("uses default, warning, and danger semantics from the actual executable set", () => {
    expect(operationConfirmationTone([preview("normal")])).toBe("default");
    expect(operationConfirmationTone([preview("sensitive", { risk_level: "Sensitive" })])).toBe("warning");
    expect(operationConfirmationTone([preview("duplicate", { is_duplicate: true })])).toBe("warning");
    expect(operationConfirmationTone([preview("system", { risk_level: "System" })])).toBe("warning");
    expect(operationConfirmationTone([preview("confirm", { requires_confirmation: true })])).toBe("warning");
    expect(operationConfirmationTone([preview("trash", { operation_type: "move_to_trash" })])).toBe("danger");
    expect(operationConfirmationTone([
      preview("sensitive-priority", { risk_level: "Sensitive" }),
      preview("trash-priority", { operation_type: "move_to_trash" })
    ])).toBe("danger");
  });

  it("localizes logical category paths without changing real nonstandard paths or internal keys", () => {
    const zh = makeTranslator("zh");
    const en = makeTranslator("en");
    expect(formatPreviewDisplayPath("C:/ZenCanvas/40_Archive/Archive/report.docx", zh)).toBe("C:\\ZenCanvas\\归档保存\\归档\\report.docx");
    expect(formatPreviewDisplayPath("/ZenCanvas/20_Areas/Work/report.docx", zh)).toBe("/ZenCanvas/长期分类/工作/report.docx");
    expect(formatPreviewDisplayPath("/ZenCanvas/90_Temporary/Downloads/setup.exe", zh)).toBe("/ZenCanvas/临时整理/下载/setup.exe");
    expect(formatPreviewDisplayPath("/ZenCanvas/40_Archive/Archive/report.docx", en)).toBe("/ZenCanvas/Archive/Archive/report.docx");
    expect(formatPreviewDisplayPath("D:/Clients/Real_Archive/report.docx", zh)).toBe("D:\\Clients\\Real_Archive\\report.docx");
    expect(formatPreviewDisplayPath("/srv/Real_Archive/report.docx", en)).toBe("/srv/Real_Archive/report.docx");
  });

  it("keeps canonical keys for grouping while exposing only localized display fields", () => {
    const groups = groupOperationPreviews([preview("archive", { target_path: "C:/ZenCanvas/40_Archive/Archive/report.docx" })], makeTranslator("zh"));
    expect(groups[0].key).toBe("40_Archive");
    expect(groups[0].rawPath).toBe("40_Archive");
    expect(groups[0].displayName).toBe("归档保存");
    expect(groups[0].displayPath).not.toContain("40_Archive");
    expect(groups[0].subgroups[0].displayPath).toBe("归档保存 / 归档");

    const realGroups = groupOperationPreviews([preview("real", { target_path: "D:/Clients/Real_Archive/report.docx" })], makeTranslator("zh"));
    expect(realGroups[0].key).toContain("real:");
    expect(realGroups[0].displayName).toBe("Real Archive");
    expect(realGroups[0].displayPath).toBe("D:\\Clients\\Real_Archive");
  });

  it("recognizes the Fylura organization root when formatting new preview paths", () => {
    const groups = groupOperationPreviews(
      [preview("fylura-root", { target_path: "C:/Users/Zen/Fylura/20_Areas/Work/report.docx" })],
      makeTranslator("zh"),
    );
    expect(groups[0].key).toBe("20_Areas");
    expect(groups[0].displayPath).toBe("长期分类");
  });

  it("does not expose internal directory keys in rendered Preview text, title, or labels", () => {
    const markup = renderToStaticMarkup(createElement(PreviewFileRow, {
      preview: preview("localized", { target_path: "C:/ZenCanvas/40_Archive/Archive/report.docx" }),
      isSelected: true,
      toggle: () => undefined,
      onRenamePreview: () => undefined,
      t: makeTranslator("zh")
    }));
    expect(markup).not.toContain("40_Archive");
    expect(markup).toContain("归档保存");
  });

  it("provides a responsive virtual group list and mounted-row ARIA contract", () => {
    const view = read("src/views/organize/OrganizeSuggestionsView.tsx");
    expect(view).toContain("useVirtualizer");
    expect(view).toContain("max-[1100px]:min-h-[320px]");
    expect(view).toContain("SideSheet");
    expect(view).toContain("aria-activedescendant={mountedActiveId}");
    expect(view).toContain("virtualRows.some");
  });

  it("preserves ordinary and batch Space behavior and AI user-correction protection", () => {
    const view = read("src/views/organize/OrganizeSuggestionsView.tsx");
    const organization = read("src-tauri/src/db/queries/organization/mod.rs");
    expect(view).toContain('event.key === " "');
    expect(view).toContain("updateGroupDecision");
    expect(view).toContain("validateOrganizeFileNameForOriginal");
    expect(view).toContain("analyzeMissing");
    expect(organization).toContain("enqueue_managed_ai_for_library_files");
    expect(organization).not.toContain("allow_overwrite_user_corrections");
  });

  it("uses translated group-first copy and item source/target details", () => {
    const view = read("src/views/organize/OrganizeSuggestionsView.tsx");
    const zh = makeTranslator("zh");
    const en = makeTranslator("en");
    for (const key of ["organizePlanTab", "organizeNeedsDecisionTab", "organizeCannotProcessTab", "organizeGroupItemFrom", "organizeGroupItemTo", "organizeReviewExecution"] as const) {
      expect(zh(key)).not.toBe(key);
      expect(en(key)).not.toBe(key);
    }
    expect(view).toContain("organizeReviewReasonLowConfidence");
    expect(view).toContain("organizeGroupReasonBlocked");
    expect(view).not.toContain("Plans are durable review artifacts");
    expect(view).not.toContain("No delete or trash operation is permitted");
  });

  it("keeps dialog focus semantics, tabular numbers, token tones, and native confirms out", () => {
    const dialog = read("src/views/shared/ui.ts");
    const timeline = read("src/views/timeline/TimelineView.tsx");
    expect(dialog).toContain("ModalPortal");
    expect(dialog).toContain("initialFocusRef: cancelRef");
    expect(dialog).toContain("onEscape");
    expect(dialog).toContain("bg-[var(--zc-overlay)]");
    expect(dialog).toContain("buttonWarning");
    expect(timeline).toContain("tabular-nums");
    for (const source of [dialog, timeline, read("src/views/organize/OrganizeSuggestionsView.tsx")]) {
      expect(source).not.toContain("window.confirm");
      expect(source).not.toContain("globalThis.confirm");
      expect(source).not.toContain("scale-");
    }
  });
});
