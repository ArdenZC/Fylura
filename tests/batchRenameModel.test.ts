import { describe, expect, it } from "vitest";
import type { OperationPreview } from "../src/types/domain";
import {
  buildBatchRenameName,
  collectExecutableBatchRenamePreviews,
  duplicateBatchRenameFileIds,
  isBatchRenameSelectionEligible,
  isCurrentExecutableBatchRenamePreview,
  keepBothCandidate,
  validateBatchRenameName,
  type BatchRenamePreviewRow,
  type BatchRenameTemplate
} from "../src/views/fileLibrary/library/batchRenameModel";

const template = (mode: BatchRenameTemplate["mode"], values: Partial<BatchRenameTemplate> = {}): BatchRenameTemplate => ({
  mode,
  prefix: "",
  start: 1,
  width: 3,
  find: "",
  replacement: "",
  pattern: "",
  ...values
});

function preview(fileId: string, name: string, overrides: Partial<OperationPreview> = {}): OperationPreview {
  return {
    id: `preview-${fileId}`,
    fileId,
    operation_type: "rename",
    source_path: `H:/B1-fixture/${fileId}.txt`,
    target_path: `H:/B1-fixture/${name}`,
    old_name: `${fileId}.txt`,
    new_name: name,
    status: "pending",
    risk_level: "Normal",
    confidence: 1,
    requires_confirmation: true,
    reason: "batch rename test fixture",
    is_executable: true,
    ...overrides
  };
}

const rows: BatchRenamePreviewRow[] = [
  { fileId: "a", name: "renamed-a.txt", hasLocalError: false },
  { fileId: "b", name: "renamed-b.txt", hasLocalError: false }
];

describe("FL-19 batch rename model", () => {
  it("numbers files in order and preserves the final extension", () => {
    expect(buildBatchRenameName("annual.report.pdf", 1, template("number", { prefix: "客户-", start: 7, width: 3 })))
      .toBe("客户-008 annual.report.pdf");
  });

  it("replaces Unicode text in the stem while preserving the extension", () => {
    expect(buildBatchRenameName("旅行-夏日.jpeg", 0, template("replace", { find: "夏日", replacement: "秋天" })))
      .toBe("旅行-秋天.jpeg");
  });

  it("applies regular-expression replacement to the stem and preserves the extension", () => {
    expect(buildBatchRenameName("报告-2026.backup.zip", 0, template("regex", { pattern: "报告-(\\d+)", replacement: "档案-$1" })))
      .toBe("档案-2026.backup.zip");
  });

  it("rejects invalid, unsafe, and unchanged names and builds Keep Both names", () => {
    expect(validateBatchRenameName("old.txt", "  ")).toBe("empty");
    expect(validateBatchRenameName("old.txt", "../new.txt")).toBe("unsafe");
    expect(validateBatchRenameName("old.txt", "OLD.TXT")).toBe("unchanged");
    expect(keepBothCandidate("报告.final.zip", 2)).toBe("报告.final (2).zip");
  });

  it("finds normalized case-insensitive duplicate targets and clears the conflict when one row is skipped", () => {
    const duplicateRows: BatchRenamePreviewRow[] = [
      { fileId: "a", name: "Résumé.txt", hasLocalError: false },
      { fileId: "b", name: "RE\u0301SUME\u0301.TXT", hasLocalError: false },
      { fileId: "c", name: "safe.txt", hasLocalError: true }
    ];
    expect(duplicateBatchRenameFileIds(duplicateRows, new Set())).toEqual(new Set(["a", "b"]));
    expect(duplicateBatchRenameFileIds(duplicateRows, new Set(["b"]))).toEqual(new Set());
  });

  it("accepts only an explicit loaded selection of 2 through 500 regular current files", () => {
    const two = [{ id: "a", isDirectory: false, isStale: false }, { id: "b", isDirectory: false, isStale: false }];
    expect(isBatchRenameSelectionEligible("explicit", ["a", "b"], two, true)).toBe(true);
    expect(isBatchRenameSelectionEligible("all_matching", ["a", "b"], two, true)).toBe(false);
    expect(isBatchRenameSelectionEligible("explicit", ["a"], [two[0]!], true)).toBe(false);
    expect(isBatchRenameSelectionEligible("explicit", ["a", "b"], [{ ...two[0]!, isDirectory: true }, two[1]!], true)).toBe(false);
    expect(isBatchRenameSelectionEligible("explicit", ["a", "b"], [{ ...two[0]!, isStale: true }, two[1]!], true)).toBe(false);
    expect(isBatchRenameSelectionEligible("explicit", ["a", "b"], [two[0]!, { ...two[1]!, id: "other" }], true)).toBe(false);
    expect(isBatchRenameSelectionEligible("explicit", ["a", "a"], two, true)).toBe(false);
    expect(isBatchRenameSelectionEligible("explicit", ["a", "b"], two, false)).toBe(false);
    const fiveHundred = Array.from({ length: 500 }, (_, index) => ({ id: `f${index}`, isDirectory: false, isStale: false }));
    expect(isBatchRenameSelectionEligible("explicit", fiveHundred.map((file) => file.id), fiveHundred, true)).toBe(true);
    const fiveHundredOne = [...fiveHundred, { id: "f500", isDirectory: false, isStale: false }];
    expect(isBatchRenameSelectionEligible("explicit", fiveHundredOne.map((file) => file.id), fiveHundredOne, true)).toBe(false);
  });

  it("does not hand off a mixture of executable and blocked previews", () => {
    const blocked = preview("b", "renamed-b.txt", { is_executable: false, blocking_reason: "manual_file_target_exists" });
    expect(collectExecutableBatchRenamePreviews(rows, new Set(), { a: preview("a", rows[0]!.name), b: blocked }, {})).toBeNull();
  });

  it("requires an explicit skip and then hands off only executable non-skipped rows", () => {
    const executable = preview("a", rows[0]!.name);
    const blocked = preview("b", rows[1]!.name, { is_executable: false, blocking_reason: "manual_file_target_exists" });
    expect(collectExecutableBatchRenamePreviews(rows, new Set(["b"]), { a: executable, b: blocked }, {})).toEqual([executable]);
  });

  it("rejects local validation errors, preview errors, and incomplete previews", () => {
    expect(collectExecutableBatchRenamePreviews([{ ...rows[0]!, hasLocalError: true }], new Set(), { a: preview("a", rows[0]!.name) }, {})).toBeNull();
    expect(collectExecutableBatchRenamePreviews([rows[0]!], new Set(), {}, {})).toBeNull();
    expect(collectExecutableBatchRenamePreviews([rows[0]!], new Set(), { a: preview("a", rows[0]!.name) }, { a: "source changed" })).toBeNull();
  });

  it("rejects edited-name mismatches and non-pending stale previews", () => {
    const oldNamePreview = preview("a", "previous-name.txt");
    expect(isCurrentExecutableBatchRenamePreview(oldNamePreview, "a", rows[0]!.name)).toBe(false);
    expect(collectExecutableBatchRenamePreviews([rows[0]!], new Set(), { a: oldNamePreview }, {})).toBeNull();
    expect(isCurrentExecutableBatchRenamePreview(preview("a", rows[0]!.name, { status: "success" }), "a", rows[0]!.name)).toBe(false);
    expect(isCurrentExecutableBatchRenamePreview(preview("a", rows[0]!.name, { status: "failed" }), "a", rows[0]!.name)).toBe(false);
    expect(isCurrentExecutableBatchRenamePreview(preview("other", rows[0]!.name), "a", rows[0]!.name)).toBe(false);
    expect(isCurrentExecutableBatchRenamePreview(preview("a", rows[0]!.name, { operation_type: "move" }), "a", rows[0]!.name)).toBe(false);
  });

  it("requires at least one explicit non-skipped row", () => {
    expect(collectExecutableBatchRenamePreviews(rows, new Set(["a", "b"]), { a: preview("a", rows[0]!.name), b: preview("b", rows[1]!.name) }, {})).toBeNull();
  });
});
