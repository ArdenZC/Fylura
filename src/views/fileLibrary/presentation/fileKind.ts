import type { PresentationEntry } from "./contracts";

/** Presentation-only file-kind token. Shared by list, grid, columns and the
 * Finder preview column so every surface colours the same extension alike. */
const FILE_KIND_BY_EXTENSION: Record<string, string> = {
  pdf: "pdf",
  doc: "doc", docx: "doc", rtf: "doc", txt: "doc", md: "doc", key: "deck",
  xls: "sheet", xlsx: "sheet", csv: "sheet", numbers: "sheet",
  ppt: "deck", pptx: "deck",
  png: "img", jpg: "img", jpeg: "img", gif: "img", webp: "img", heic: "img", svg: "img",
  mp4: "video", mov: "video", mkv: "video", avi: "video", webm: "video",
  mp3: "audio", wav: "audio", flac: "audio", aac: "audio", m4a: "audio",
  js: "code", ts: "code", tsx: "code", jsx: "code", json: "code", py: "code", rs: "code", css: "code", html: "code",
  zip: "zip", gz: "zip", tar: "zip", "7z": "zip", rar: "zip", dmg: "zip"
};

export function fileKindToken(entry: PresentationEntry, isDirectory: boolean) {
  if (isDirectory) return "folder";
  const extension = (entry.extension ?? "").toLocaleLowerCase();
  return FILE_KIND_BY_EXTENSION[extension] ?? "file";
}
