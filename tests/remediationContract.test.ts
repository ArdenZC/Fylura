import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = (path: string) => readFileSync(path, "utf8");
const cleanup = source("src-tauri/src/storage_analyzer.rs");
const cleanupAI = source("src-tauri/src/ai/cleanup.rs");
const settings = source("src-tauri/src/settings.rs");
const aiSettings = source("src-tauri/src/ai/settings.rs");
const aiValidation = source("src-tauri/src/ai/openai_compatible.rs") + source("src-tauri/src/ai/settings.rs");
const fileOps = [
  source("src-tauri/src/file_ops.rs"),
  source("src-tauri/src/file_ops/recovery.rs"),
  source("src-tauri/src/file_ops/restore.rs")
].join("\n");
const schema = source("src-tauri/src/db/schema.rs");
const safeTrash = cleanup;
const dedupe = source("src-tauri/src/dedupe.rs");
const ids = source("src-tauri/src/ids.rs");
const capabilities = source("src-tauri/src/runtime_capabilities.rs");
const tauriCapabilities = source("src-tauri/capabilities/default.json");
const cargo = source("src-tauri/Cargo.toml");
const cleanupApi = source("src/api/cleanupApi.ts");
const settingsApi = source("src/api/settingsApi.ts");
const packageJson = source("package.json");
const workflows = source(".github/workflows/ci.yml") + source(".github/workflows/release-build.yml");
const releaseWorkflow = source(".github/workflows/release-build.yml");
const supportedPlatforms = source("docs/security/SUPPORTED_PLATFORMS.md");
const platformSupport = source("src-tauri/src/fs_safety/platform_support.rs");
const sourceClaim = source("src-tauri/src/fs_safety/source_claim.rs");
const sourceClaimProduction = sourceClaim.split("#[cfg(test)]")[0];
const verifiedDirectory = source("src-tauri/src/fs_safety/verified_directory.rs");
const atomicMove = source("src-tauri/src/fs_safety/atomic_move.rs");

describe("remediation contracts", () => {
  it("requires jobId at every cleanup candidate command boundary", () => {
    for (const command of [
      "preview_cleanup_candidates",
      "preview_cleanup_operations",
      "move_cleanup_candidates_to_safe_trash"
    ]) {
      expect(cleanup).toMatch(new RegExp(`pub ${command === "preview_cleanup_candidates" ? "" : "async "}fn ${command}[\\s\\S]{0,260}?job_id: String`));
      expect(cleanup).toMatch(new RegExp(`pub ${command === "preview_cleanup_candidates" ? "" : "async "}fn ${command}[\\s\\S]{0,360}?selections: Vec<CleanupFindingSelection>`));
    }
    expect(cleanupAI).toMatch(/pub async fn analyze_cleanup_candidates_with_ai[\s\S]{0,220}?job_id: String/);
    for (const method of [
      "previewCleanupCandidates",
      "previewCleanupOperations",
      "moveCleanupCandidatesToSafeTrash"
    ]) {
      expect(cleanupApi).toMatch(new RegExp(`${method}\\(jobId: string, selections: CleanupFindingSelection\\[\\]\\)`));
    }
    expect(cleanupApi).not.toContain("moveCleanupCandidatesToTrash");
    expect(cleanup).not.toContain("move_cleanup_candidates_to_trash");
    expect(cleanup).not.toContain("move_path_to_system_trash_with_safety");
    expect(source("src-tauri/build.rs")).not.toContain("move_cleanup_candidates_to_trash");
    expect(source("src-tauri/src/main.rs")).not.toContain("move_cleanup_candidates_to_trash");
    expect(tauriCapabilities).not.toContain("allow-move-cleanup-candidates-to-trash");
  });

  it("keeps cleanup candidates job-scoped and cross-job resolution atomic", () => {
    expect(cleanup).toContain("jobs: Mutex<HashMap<String, StorageCleanupJob>>");
    expect(cleanup).toContain("job.candidates_by_id.get(id).cloned().ok_or_else");
    expect(cleanup).toContain("does not belong to job {job_id}");
    expect(cleanup).toContain("finding.revision != selection.expected_revision");
    expect(cleanup).toContain("finding_identity_matches(db, &finding)");
    expect(cleanup).toContain("Review finding requires explicit acknowledged confirmation");
    expect(cleanup).not.toMatch(/latest_candidates/i);
  });

  it("requires settings CAS revision and rejects stale writers", () => {
    expect(settings).toMatch(/pub struct SaveSettingsRequest[\s\S]*expected_revision: i64/);
    expect(settings).toContain("WHERE key = ?2 AND revision = ?3");
    expect(settings).toContain("SettingsError::RevisionConflict");
    expect(settingsApi).toContain("saveSettings(request: SaveSettingsRequest)");
  });

  it("requires file identity for operation and Safe Trash recovery", () => {
    expect(fileOps).toContain("source_quick_hash");
    expect(fileOps).toContain("target_platform_file_id");
    expect(fileOps).toContain("mark_restore_manual_review");
    expect(safeTrash).toContain("trash_quick_hash");
    expect(safeTrash).toContain("trash_platform_file_id");
    expect(schema).toContain("legacy_unverified");
    expect(safeTrash).toContain("manual_review");
  });

  it("isolates dedupe cancellation per job", () => {
    expect(dedupe).toContain("jobs: HashMap<String, DedupeJob>");
    expect(dedupe).toContain("scan_to_dedupe: HashMap<String, String>");
    expect(dedupe).not.toContain("DEDUPE_CANCEL_REQUESTED");
    expect(dedupe).not.toContain("DEDUPE_RUNNING");
  });

  it("uses collision-resistant UUID job IDs", () => {
    expect(ids).toContain("uuid::Uuid::now_v7()");
    expect(ids).toContain("concurrent_uuid_job_ids_do_not_collide");
    expect(ids).not.toMatch(/SystemTime|UNIX_EPOCH|DefaultHasher/);
  });

  it("keeps AI credential changes explicit, verified, and secret-free", () => {
    expect(aiSettings).toContain("ApiKeyAction::Preserve");
    expect(aiSettings).toContain("verify_credential_change(");
    expect(aiSettings).toContain("credential read-back verification failed");
    expect(aiSettings).toContain("persisted.api_key.clear()");
    expect(aiSettings).toContain("failed to read API key from system credential store");
    expect(aiSettings).not.toMatch(/OnceLock\s*<\s*Mutex\s*<\s*String\s*>\s*>/);
  });

  it("hides debug controls in production while retaining real AI", () => {
    expect(capabilities).toContain("production_capabilities_hide_ai_debug_without_disabling_real_ai");
    expect(capabilities).toContain("assert!(!release.ai_debug_available)");
    expect(capabilities).toContain("assert!(release.real_ai_classification_available)");
  });

  it("validates cloud URLs and rejects reserved Extra Body fields", () => {
    expect(aiValidation).toContain("Release builds require HTTPS for non-local AI providers");
    expect(aiValidation).toContain("extra_body_cannot_override_reserved_fields");
    for (const field of ["model", "messages", "stream", "temperature", "max_tokens", "response_format", "thinking"]) {
      expect(aiValidation).toContain(`\"${field}\"`);
    }
  });

  it("keeps revision and identity migrations in the current schema", () => {
    expect(schema).toMatch(/CURRENT_SCHEMA_VERSION: i32 = (?:2[0-9]|[3-9][0-9])/);
    expect(schema).toContain("ALTER TABLE app_settings ADD COLUMN revision");
    expect(schema).toContain("ALTER TABLE operation_logs ADD COLUMN source_quick_hash");
    expect(schema).toContain("ALTER TABLE cleanup_trash_items ADD COLUMN identity_status");
    expect(schema).toContain("restore_status = 'manual_review'");
  });

  it("does not suppress the remediated RustSec advisories", () => {
    const auditSurfaces = `${packageJson}\n${workflows}`;
    expect(auditSurfaces).not.toContain("--ignore RUSTSEC-2026-0194");
    expect(auditSurfaces).not.toContain("--ignore RUSTSEC-2026-0195");
    expect(packageJson).toContain("cargo audit --file src-tauri/Cargo.lock");
  });

  it("keeps release distribution unsigned and binds publication to final artifacts", () => {
    expect(releaseWorkflow).toContain("Distribution model: UNSIGNED");
    expect(releaseWorkflow).toContain("Windows Authenticode: NOT PROVIDED");
    expect(releaseWorkflow).toContain("macOS Developer ID: NOT PROVIDED");
    expect(releaseWorkflow).toContain("Apple notarization: NOT PROVIDED");
    expect(releaseWorkflow).toContain("Stapling: NOT PROVIDED");
    expect(releaseWorkflow).toContain("Signing/notarization is intentionally deferred by product decision.");
    for (const unsupportedClaim of [
      "Windows Authenticode: PASS",
      "macOS Developer ID: PASS",
      "Apple notarization: PASS",
      "Stapling: PASS",
    ]) {
      expect(releaseWorkflow).not.toContain(unsupportedClaim);
    }
    expect(releaseWorkflow).toContain("run: npm run build -- --no-sign");
    expect(releaseWorkflow).not.toContain("APPLE_CERTIFICATE");
    expect(releaseWorkflow).not.toContain("APPLE_SIGNING_IDENTITY");
    expect(releaseWorkflow).not.toContain("codesign");
    expect(releaseWorkflow).toContain("Verify checkout provenance and version metadata");
    expect(releaseWorkflow).toContain("Verify final release artifacts and checksums");
    expect(releaseWorkflow).toContain("Get-FileHash -LiteralPath");
    expect(releaseWorkflow).toContain("basename \"$dmg\"");
    expect(releaseWorkflow).toContain("Verify SBOM outputs");
    expect(releaseWorkflow).toContain("if-no-files-found: error");
    expect(releaseWorkflow).toMatch(/release:\s*[\s\S]*needs: build/);
    expect(releaseWorkflow).toContain("fail_on_unmatched_files: true");
  });

  it("keeps the supported product policy Windows/macOS-only", () => {
    expect(supportedPlatforms).toContain("Windows");
    expect(supportedPlatforms).toContain("macOS");
    expect(supportedPlatforms).toMatch(/Linux is not a supported product platform/);
    expect(workflows).toContain("runs-on: windows-latest");
    expect(workflows).toContain("runs-on: macos-latest");
    expect(workflows).toContain("Frontend and format quality");
    expect(workflows).toContain("runs-on: ubuntu-latest");
    expect(workflows).not.toContain("Linux Tauri dependencies");
    expect(cargo).not.toContain('target_os = "linux"');
    expect(fileOps).not.toContain("xdg-open");
    expect(platformSupport).toContain("unsupported_platform_linux");
    expect(fileOps).toContain("ensure_supported_file_mutation");
  });

  it("binds mutation claims to verified directories and stable recovery phases", () => {
    expect(sourceClaim).toContain("atomic_source_binding_unsupported");
    expect(sourceClaim).toContain("AfterJournalPreparedBeforeClaim");
    expect(sourceClaim).toContain("AfterClaimVerifiedBeforeTargetCommit");
    expect(sourceClaim).toContain("AfterTargetCommitBeforeSourceCleanup");
    expect(sourceClaim).toContain("AfterSourceCleanupBeforeJournalComplete");
    expect(sourceClaimProduction).not.toContain("to_string_lossy");
    expect(verifiedDirectory).toContain("ensure_unchanged");
    expect(verifiedDirectory).toContain("DirectoryIdentity");
    expect(atomicMove).toContain("CrossVolumeDirectoryMoveUnsupported");
    expect(atomicMove).toContain("claim_source_at");
  });
});
