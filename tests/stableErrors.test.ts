import { describe, expect, it } from "vitest";
import { makeTranslator } from "../src/i18n";
import { localizedStableError } from "../src/utils/viewHelpers";

describe("stable error codes", () => {
  it("maps filesystem and credential codes through the active locale", () => {
    const zh = makeTranslator("zh");
    const en = makeTranslator("en");

    expect(localizedStableError(new Error("source_changed"), zh)).toContain("文件");
    expect(localizedStableError("atomic_noreplace_unsupported", en)).toContain("filesystem");
    expect(localizedStableError("credential_transaction_lock_poisoned", en)).toContain("credential");
    expect(localizedStableError("watcher_retry_exhausted", zh)).toContain("文件监听");
    expect(localizedStableError("source_claim_recovery_required: claim", zh)).toContain("待恢复");
    expect(localizedStableError("unsupported_platform_linux", en)).toContain("unsupported");
    expect(localizedStableError("target_committed_source_delete_failed: denied", en)).toContain("manual review");
    expect(localizedStableError("copy_verification_failed", en)).toContain("identity verification");
    expect(localizedStableError("target_parent_durability_unknown", zh)).toContain("持久化");
    expect(localizedStableError("macos_file_mutation_source_binding_unsupported", en)).toContain("source-handle");
    expect(localizedStableError("staging_identity_changed", zh)).toContain("暂存对象");
    expect(localizedStableError("staging_handle_commit_unsupported", en)).toContain("staging file handle");
    expect(localizedStableError("target_committed_durability_unknown", zh)).toContain("目标已提交");
    expect(localizedStableError("target_committed_identity_mismatch", en)).toContain("post-commit identity");
    expect(localizedStableError("target_committed_source_cleanup_pending", zh)).toContain("清理仍待完成");
    expect(localizedStableError("restore_pending_reconciliation", en)).toContain("startup reconciliation");
    expect(localizedStableError("claim_identity_mismatch", zh)).toContain("身份不一致");
    expect(localizedStableError("manual_review_required", en)).toContain("manual review");
    expect(localizedStableError("system_trash_source_binding_unsupported", zh)).toContain("系统回收站");
  });

  it("preserves unknown technical details for diagnostics", () => {
    expect(localizedStableError("provider_timeout", makeTranslator("en"))).toBe("provider_timeout");
  });

  it("shows only the safe Settings support classification", () => {
    const en = makeTranslator("en");
    expect(localizedStableError("settings_save_failure:watcher_root_sync_failure", en))
      .toBe("Failed to save settings (Support code: watcher_root_sync_failure)");
    expect(localizedStableError("settings_save_failure:rollback_reconciliation_failure", en))
      .toContain("rollback_reconciliation_failure");
  });
});
