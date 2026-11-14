import { describe, expect, it } from "vitest";
import { Archive, File, FileText, Film, Folder, Image, Music, Table } from "lucide-react";
import { previewTypeIcon } from "../src/views/fileLibrary/preview/ZenQuickPreviewSurface";

describe("preview identity glyph", () => {
  it("maps file families without inspecting content", () => {
    expect(previewTypeIcon(undefined, undefined, "directory")).toBe(Folder);
    expect(previewTypeIcon("png", undefined, "file")).toBe(Image);
    expect(previewTypeIcon(undefined, "image/png", "file")).toBe(Image);
    expect(previewTypeIcon("mp4", undefined, "file")).toBe(Film);
    expect(previewTypeIcon("mp3", undefined, "file")).toBe(Music);
    expect(previewTypeIcon("zip", undefined, "file")).toBe(Archive);
    expect(previewTypeIcon("xlsx", undefined, "file")).toBe(Table);
    expect(previewTypeIcon("pdf", undefined, "file")).toBe(FileText);
    expect(previewTypeIcon("rs", undefined, "file")).toBe(File);
    expect(previewTypeIcon("PNG", undefined, "file")).toBe(Image);
  });
});
