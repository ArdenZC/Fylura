import type { OperationPreview } from "../../../types/domain";

export type BatchRenameMode = "number" | "replace" | "regex";

export type BatchRenameTemplate = {
  mode: BatchRenameMode;
  prefix: string;
  start: number;
  width: number;
  find: string;
  replacement: string;
  pattern: string;
};

export const BATCH_RENAME_LIMIT = 500;

export type BatchRenameSelectionFile = {
  id: string;
  isDirectory: boolean;
  isStale: boolean;
};

export function isBatchRenameSelectionEligible(
  selectionKind: "explicit" | "all_matching" | null | undefined,
  selectedIds: readonly string[],
  files: readonly BatchRenameSelectionFile[],
  renameAvailable: boolean
): boolean {
  if (!renameAvailable || selectionKind !== "explicit" || selectedIds.length < 2 || selectedIds.length > BATCH_RENAME_LIMIT) return false;
  if (files.length !== selectedIds.length || files.some((file) => file.isDirectory || file.isStale)) return false;
  const selected = new Set(selectedIds);
  return selected.size === selectedIds.length && files.every((file) => selected.has(file.id));
}

export type BatchRenamePreviewRow = {
  fileId: string;
  name: string;
  hasLocalError: boolean;
};

export function duplicateBatchRenameFileIds(
  rows: readonly BatchRenamePreviewRow[],
  skippedIds: ReadonlySet<string>
): Set<string> {
  const idsByTarget = new Map<string, string[]>();
  for (const row of rows) {
    if (skippedIds.has(row.fileId) || row.hasLocalError || !row.name) continue;
    const key = row.name.normalize("NFC").toLocaleLowerCase("en-US");
    const ids = idsByTarget.get(key);
    if (ids) ids.push(row.fileId); else idsByTarget.set(key, [row.fileId]);
  }
  return new Set([...idsByTarget.values()].filter((ids) => ids.length > 1).flat());
}

export function isCurrentExecutableBatchRenamePreview(
  preview: OperationPreview | undefined,
  fileId: string,
  name: string
): preview is OperationPreview {
  return preview !== undefined
    && preview.fileId === fileId
    && preview.operation_type === "rename"
    && preview.status === "pending"
    && preview.new_name === name
    && preview.is_executable !== false
    && !preview.blocking_reason;
}

/** Return null until every non-skipped row has a current executable backend preview. */
export function collectExecutableBatchRenamePreviews(
  rows: readonly BatchRenamePreviewRow[],
  skippedIds: ReadonlySet<string>,
  previews: Readonly<Record<string, OperationPreview | undefined>>,
  previewErrors: Readonly<Record<string, string | undefined>>
): OperationPreview[] | null {
  const included = rows.filter((row) => !skippedIds.has(row.fileId));
  if (!included.length) return null;

  const result: OperationPreview[] = [];
  for (const row of included) {
    const preview = previews[row.fileId];
    if (row.hasLocalError || previewErrors[row.fileId] || !isCurrentExecutableBatchRenamePreview(preview, row.fileId, row.name)) {
      return null;
    }
    result.push(preview);
  }
  return result;
}

function splitExtension(name: string) {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? [name.slice(0, dot), name.slice(dot)] as const : [name, ""] as const;
}

export function buildBatchRenameName(name: string, index: number, template: BatchRenameTemplate): string {
  const [stem, extension] = splitExtension(name);
  if (template.mode === "number") {
    return `${template.prefix}${String(template.start + index).padStart(template.width, "0")} ${stem}${extension}`;
  }
  if (template.mode === "replace") {
    return `${stem.split(template.find).join(template.replacement)}${extension}`;
  }
  return `${stem.replace(new RegExp(template.pattern, "g"), template.replacement)}${extension}`;
}

export function validateBatchRenameName(oldName: string, newName: string): "empty" | "unsafe" | "unchanged" | null {
  if (!newName.trim()) return "empty";
  if (newName !== newName.trim() || /[\\/\u0000-\u001f]/.test(newName)) return "unsafe";
  if (oldName.toLocaleLowerCase("en-US") === newName.toLocaleLowerCase("en-US")) return "unchanged";
  return null;
}

export function keepBothCandidate(name: string, ordinal: number): string {
  const [stem, extension] = splitExtension(name);
  return `${stem} (${ordinal})${extension}`;
}
