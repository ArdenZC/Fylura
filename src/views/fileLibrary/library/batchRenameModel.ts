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
