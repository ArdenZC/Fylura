import { expect, it } from "vitest";
import { makeTranslator } from "../src/i18n/dictionary";

it("discloses whole selected disk, persistent journal metadata and privileges in both languages", () => {
  const zh = makeTranslator("zh")("globalIndexFullDiskConfirm");
  const en = makeTranslator("en")("globalIndexFullDiskConfirm");
  for (const phrase of ["整个", "NTFS USN", "持久", "卷级", "管理员", "服务权限", "Google Drive", "未授权"]) expect(zh).toContain(phrase);
  for (const phrase of ["entire selected", "NTFS USN", "persistent", "volume-level", "administrator", "service privileges", "Google Drive", "Unauthorized"]) expect(en).toContain(phrase);
});
