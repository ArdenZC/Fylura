import { createAutomationMock } from "./automationMock";
import type {
  AIConnectionTestResult,
  AIDebugClassificationResult,
  AIModelInfo,
  AIProductFeatureReadiness,
  AIProviderPreset,
  AIRequestTrace,
  AISettings,
  AIReadinessState,
  AddManagedScopeRequest,
  AiManagementStatus,
  AppSettings,
  ClassificationCorrectionRequest,
  CleanupRestorePreview,
  CleanupRestoreResult,
  CleanupTrashBatch,
  CleanupExecutionResult,
  CleanupPreviewItem,
  CreateLibrarySavedViewRequest,
  CreateUserTagRequest,
  DashboardStats,
  DeleteLibrarySavedViewRequest,
  DeleteUserTagRequest,
  DedupeGroup,
  DedupeGroupMember,
  DedupeGroupPage,
  DedupeRun,
  AnalysisDetector,
  AnalysisDetectorDescriptor,
  AnalysisFinding,
  AnalysisFindingDecision,
  AnalysisFindingEvidence,
  AnalysisFindingPage,
  AnalysisRun,
  StartAnalysisRunRequest,
  ExecuteOperationResult,
  FileLibraryFilters,
  FileLibraryDetail,
  FileLibrarySelectionSummary,
  FileLibrarySummary,
  FileQueryRequestV2,
  FileQueryResponseV2,
  FileQueryResult,
  FileRecord,
  GlobalIndexSource,
  GlobalIndexStatus,
  GlobalSearchRequest,
  GlobalSearchResponse,
  GlobalSearchResult,
  GlobalSearchSourceHealth,
  LibrarySavedView,
  LibrarySelectionV1,
  LibraryScope,
  ManagedScope,
  OperationLog,
  OperationPreview,
  OperationPreviewResult,
  RecoveryActionResult,
  OrganizationPlan,
  OrganizationPlanDryRun,
  OrganizationPlanEffectiveSummary,
  OrganizationPlanGroupItemPage,
  OrganizationPlanGroupPage,
  OrganizationPlanGroupSummary,
  OrganizationPlanItem,
  RestoreMovesResult,
  Rule,
  RuleDraftV2,
  RuleProposal,
  RuleProposalImpact,
  RuleExecutionMode,
  RuleExecutionSummary,
  ContentArtifact,
  ContentScopePolicy,
  MutateFileUserTagsRequest,
  MutateFileUserTagsResult,
  SaveSettingsRequest,
  VersionedAppSettings,
  UpdateLibrarySavedViewRequest,
  UpdateUserTagRequest,
  UserTag,
  UpdateManagedScopePolicyRequest,
  StorageAnalysis,
  StorageCandidate,
  StorageCleanupScanStatus
} from "../types/domain";
import type { View } from "../types/ui";
import { DEFAULT_SEARCH_HOTKEY } from "../utils/hotkeys";
import { isFileWorkspaceMockCommand, mockFileWorkspaceInvoke } from "./fileWorkspaceMockApi";
import type {
  GlobalHotkeyStatus,
  ManagedScanRequest,
  ManagedScanSnapshotDto,
  ManagedScanStartDto,
  ScanRootDto,
  ScanRunDto,
  ScanSessionDto,
  ScanSummary,
  SearchWindowSnapshot
} from "./types";

const now = "2026-07-06T09:00:00.000Z";
const W211_LIBRARY_TOTAL = 100_000;
let mockSearchWindowState: SearchWindowSnapshot = {
  sessionId: 1,
  revision: 1,
  phase: "visible_collapsed"
};

type MockCleanupRestoreState = Pick<CleanupTrashBatch["items"][number], "status" | "restoredAt" | "message">;

const mockCleanupCreatedAt = Date.now().toString();
const mockCleanupRestoreState = new Map<string, MockCleanupRestoreState>();

const mockFiles: FileRecord[] = [
  file({
    id: "mock-report",
    name: "project-report.pdf",
    path: "C:/Users/Zen/Documents/project-report.pdf",
    directory: "C:/Users/Zen/Documents",
    extension: "pdf",
    size: 2_450_000,
    file_type: "Document",
    purpose: "Work",
    lifecycle: "Active",
    confidence: 0.86
  }),
  file({
    id: "mock-archive",
    name: "old-design-assets.zip",
    path: "C:/Users/Zen/Downloads/old-design-assets.zip",
    directory: "C:/Users/Zen/Downloads",
    extension: "zip",
    size: 84_000_000,
    file_type: "ArchivePackage",
    purpose: "Archive",
    lifecycle: "Archive",
    suggested_action: "Archive",
    confidence: 0.78
  }),
  file({
    id: "mock-duplicate",
    name: "invoice-copy.pdf",
    path: "C:/Users/Zen/Desktop/invoice-copy.pdf",
    directory: "C:/Users/Zen/Desktop",
    extension: "pdf",
    size: 810_000,
    file_type: "Document",
    purpose: "Finance",
    lifecycle: "Duplicate",
    is_duplicate: true,
    suggested_action: "Review",
    requires_confirmation: true,
    confidence: 0.7
  }),
  file({
    id: "mock-private",
    name: "passport-scan.png",
    path: "C:/Users/Zen/Documents/private/passport-scan.png",
    directory: "C:/Users/Zen/Documents/private",
    extension: "png",
    size: 1_280_000,
    file_type: "Image",
    purpose: "Identity",
    lifecycle: "Sensitive",
    risk_level: "Sensitive",
    requires_confirmation: true,
    confidence: 0.91
  }),
  file({
    id: "mock-installer",
    name: "setup-helper.exe",
    path: "C:/Users/Zen/Downloads/setup-helper.exe",
    directory: "C:/Users/Zen/Downloads",
    extension: "exe",
    size: 32_000_000,
    file_type: "Installer",
    purpose: "Installer",
    lifecycle: "Disposable",
    suggested_action: "Review",
    confidence: 0.74
  })
];

function isW309IntegrationFixtureEnabled() {
  if (typeof window === "undefined") return false;
  return new URLSearchParams(window.location.search).get("w3-09-browser-fixture") === "integration";
}

function isW304ProviderFixtureEnabled() {
  if (typeof window === "undefined") return false;
  return isW309IntegrationFixtureEnabled()
    || new URLSearchParams(window.location.search).get("w3-04-browser-fixture") === "providers";
}

function isW305ProviderFixtureEnabled() {
  if (typeof window === "undefined") return false;
  return isW309IntegrationFixtureEnabled()
    || new URLSearchParams(window.location.search).get("w3-05-browser-fixture") === "providers";
}

function isW306ProviderFixtureEnabled() {
  if (typeof window === "undefined") return false;
  return isW309IntegrationFixtureEnabled()
    || new URLSearchParams(window.location.search).get("w3-06-browser-fixture") === "images";
}

function isW308ProviderFixtureEnabled() {
  if (typeof window === "undefined") return false;
  return new URLSearchParams(window.location.search).get("w3-08-browser-fixture") === "archives";
}

if (isW304ProviderFixtureEnabled()) {
  mockFiles.push(
    file({
      id: "w3-04-readme",
      name: "W3-04-hostile.md",
      path: "C:/Users/Zen/Documents/W3-04-hostile.md",
      directory: "C:/Users/Zen/Documents",
      extension: "md",
      size: 1_280,
      file_type: "Document",
      purpose: "Document",
      lifecycle: "Active",
      context: "W3-04 sanitized Markdown fixture"
    }),
    file({
      id: "w3-04-source",
      name: "preview-fixture.rs",
      path: "C:/Users/Zen/Documents/preview-fixture.rs",
      directory: "C:/Users/Zen/Documents",
      extension: "rs",
      size: 768,
      file_type: "Code",
      purpose: "Work",
      lifecycle: "Active",
      context: "W3-04 source-code fixture"
    }),
    file({
      id: "w3-04-partial",
      name: "bounded-prefix.txt",
      path: "C:/Users/Zen/Documents/bounded-prefix.txt",
      directory: "C:/Users/Zen/Documents",
      extension: "txt",
      size: 700_000,
      file_type: "Document",
      purpose: "Document",
      lifecycle: "Active",
      context: "W3-04 bounded partial fixture"
    }),
    file({
      id: "w3-04-fallback",
      name: "unsupported-preview.bin",
      path: "C:/Users/Zen/Documents/unsupported-preview.bin",
      directory: "C:/Users/Zen/Documents",
      extension: "bin",
      size: 8_192,
      file_type: "Other",
      purpose: "Document",
      lifecycle: "Active",
      context: "W3-04 metadata fallback fixture"
    })
  );
}

if (isW305ProviderFixtureEnabled()) {
  mockFiles.push(
    file({
      id: "000-w3-05-json",
      name: "structured-sample.json",
      path: "C:/Users/Zen/Documents/structured-sample.json",
      directory: "C:/Users/Zen/Documents",
      extension: "json",
      size: 1_024,
      file_type: "Document",
      purpose: "Work",
      lifecycle: "Active",
      context: "W3-05 JSON structured-tree fixture"
    }),
    file({
      id: "w3-05-yaml",
      name: "structured-config.yaml",
      path: "C:/Users/Zen/Documents/structured-config.yaml",
      directory: "C:/Users/Zen/Documents",
      extension: "yaml",
      size: 1_024,
      file_type: "Document",
      purpose: "Work",
      lifecycle: "Active",
      context: "W3-05 YAML structured-tree fixture"
    }),
    file({
      id: "w3-05-xml",
      name: "structured-markup.xml",
      path: "C:/Users/Zen/Documents/structured-markup.xml",
      directory: "C:/Users/Zen/Documents",
      extension: "xml",
      size: 1_024,
      file_type: "Document",
      purpose: "Work",
      lifecycle: "Active",
      context: "W3-05 XML structured-tree fixture"
    }),
    file({
      id: "w3-05-csv",
      name: "structured-records.csv",
      path: "C:/Users/Zen/Documents/structured-records.csv",
      directory: "C:/Users/Zen/Documents",
      extension: "csv",
      size: 1_024,
      file_type: "Document",
      purpose: "Work",
      lifecycle: "Active",
      context: "W3-05 CSV table fixture"
    }),
    file({
      id: "w3-05-tsv",
      name: "structured-records.tsv",
      path: "C:/Users/Zen/Documents/structured-records.tsv",
      directory: "C:/Users/Zen/Documents",
      extension: "tsv",
      size: 1_024,
      file_type: "Document",
      purpose: "Work",
      lifecycle: "Active",
      context: "W3-05 TSV table fixture"
    }),
    file({
      id: "w3-05-structured-partial",
      name: "structured-partial.json",
      path: "C:/Users/Zen/Documents/structured-partial.json",
      directory: "C:/Users/Zen/Documents",
      extension: "json",
      size: 700_000,
      file_type: "Document",
      purpose: "Work",
      lifecycle: "Active",
      context: "W3-05 partial structured-tree fixture"
    }),
    file({
      id: "w3-05-table-partial",
      name: "table-partial.csv",
      path: "C:/Users/Zen/Documents/table-partial.csv",
      directory: "C:/Users/Zen/Documents",
      extension: "csv",
      size: 700_000,
      file_type: "Document",
      purpose: "Work",
      lifecycle: "Active",
      context: "W3-05 partial table fixture"
    }),
    file({
      id: "w3-05-malformed",
      name: "malformed-structured.json",
      path: "C:/Users/Zen/Documents/malformed-structured.json",
      directory: "C:/Users/Zen/Documents",
      extension: "json",
      size: 128,
      file_type: "Document",
      purpose: "Work",
      lifecycle: "Active",
      context: "W3-05 malformed provider fallback fixture"
    }),
    file({
      id: "w3-05-fallback",
      name: "unsupported-structured.bin",
      path: "C:/Users/Zen/Documents/unsupported-structured.bin",
      directory: "C:/Users/Zen/Documents",
      extension: "bin",
      size: 8_192,
      file_type: "Other",
      purpose: "Document",
      lifecycle: "Active",
      context: "W3-05 metadata fallback fixture"
    })
  );
}

if (isW306ProviderFixtureEnabled()) {
  mockFiles.push(
    file({
      id: "000-w3-06-png",
      name: "image-sample.png",
      path: "C:/Users/Zen/Documents/image-sample.png",
      directory: "C:/Users/Zen/Documents",
      extension: "png",
      size: 1_024,
      file_type: "Image",
      purpose: "Work",
      lifecycle: "Active",
      context: "W3-06 bounded PNG image fixture"
    }),
    file({
      id: "w3-06-jpeg",
      name: "image-sample.jpg",
      path: "C:/Users/Zen/Documents/image-sample.jpg",
      directory: "C:/Users/Zen/Documents",
      extension: "jpg",
      size: 2_048,
      file_type: "Image",
      purpose: "Work",
      lifecycle: "Active",
      context: "W3-06 bounded JPEG image fixture"
    }),
    file({
      id: "w3-06-partial",
      name: "image-bounded.png",
      path: "C:/Users/Zen/Documents/image-bounded.png",
      directory: "C:/Users/Zen/Documents",
      extension: "png",
      size: 12_582_912,
      file_type: "Image",
      purpose: "Work",
      lifecycle: "Active",
      context: "W3-06 Partial bounded image fixture"
    }),
    file({
      id: "w3-06-corrupt",
      name: "image-corrupt.png",
      path: "C:/Users/Zen/Documents/image-corrupt.png",
      directory: "C:/Users/Zen/Documents",
      extension: "png",
      size: 96,
      file_type: "Image",
      purpose: "Document",
      lifecycle: "Active",
      context: "W3-06 corrupt image metadata fallback fixture"
    }),
    file({
      id: "w3-06-oversized",
      name: "image-oversized.png",
      path: "C:/Users/Zen/Documents/image-oversized.png",
      directory: "C:/Users/Zen/Documents",
      extension: "png",
      size: 1_048_576,
      file_type: "Image",
      purpose: "Document",
      lifecycle: "Active",
      context: "W3-06 oversized image metadata fallback fixture"
    }),
    file({
      id: "w3-06-unsupported",
      name: "image-vector.svg",
      path: "C:/Users/Zen/Documents/image-vector.svg",
      directory: "C:/Users/Zen/Documents",
      extension: "svg",
      size: 4_096,
      file_type: "Image",
      purpose: "Document",
      lifecycle: "Active",
      context: "W3-06 unsupported SVG metadata fallback fixture"
    })
  );
}

if (isW309IntegrationFixtureEnabled()) {
  mockFiles.push(
    file({
      id: "w3-09-materialization",
      name: "materialization-required.txt",
      path: "C:/Users/Zen/Documents/materialization-required.txt",
      directory: "C:/Users/Zen/Documents",
      extension: "txt",
      size: 4_096,
      file_type: "Document",
      purpose: "Work",
      lifecycle: "Active",
      context: "W3-09 terminal MaterializationRequired fixture"
    }),
    file({
      id: "w3-09-permission",
      name: "permission-denied.txt",
      path: "C:/Users/Zen/Documents/permission-denied.txt",
      directory: "C:/Users/Zen/Documents",
      extension: "txt",
      size: 4_096,
      file_type: "Document",
      purpose: "Work",
      lifecycle: "Active",
      context: "W3-09 terminal PermissionDenied fixture"
    }),
    file({
      id: "w3-09-unavailable",
      name: "source-unavailable.txt",
      path: "C:/Users/Zen/Documents/source-unavailable.txt",
      directory: "C:/Users/Zen/Documents",
      extension: "txt",
      size: 4_096,
      file_type: "Document",
      purpose: "Work",
      lifecycle: "Active",
      context: "W3-09 terminal SourceUnavailable fixture"
    }),
    file({
      id: "w3-09-identity",
      name: "identity-changed.txt",
      path: "C:/Users/Zen/Documents/identity-changed.txt",
      directory: "C:/Users/Zen/Documents",
      extension: "txt",
      size: 4_096,
      file_type: "Document",
      purpose: "Work",
      lifecycle: "Active",
      context: "W3-09 terminal IdentityChanged fixture"
    })
  );
}

if (isW308ProviderFixtureEnabled()) {
  mockFiles.push(
    file({
      id: "w3-08-sample",
      name: "archive-sample.zip",
      path: "C:/Users/Zen/Documents/archive-sample.zip",
      directory: "C:/Users/Zen/Documents",
      extension: "zip",
      size: 4_096,
      file_type: "Other",
      purpose: "Work",
      lifecycle: "Active",
      context: "W3-08 complete ZIP central-directory metadata fixture"
    }),
    file({
      id: "w3-08-empty",
      name: "archive-empty.zip",
      path: "C:/Users/Zen/Documents/archive-empty.zip",
      directory: "C:/Users/Zen/Documents",
      extension: "zip",
      size: 22,
      file_type: "Other",
      purpose: "Work",
      lifecycle: "Active",
      context: "W3-08 empty ZIP metadata fixture"
    }),
    file({
      id: "w3-08-partial",
      name: "archive-partial.zip",
      path: "C:/Users/Zen/Documents/archive-partial.zip",
      directory: "C:/Users/Zen/Documents",
      extension: "zip",
      size: 12_582_912,
      file_type: "Other",
      purpose: "Work",
      lifecycle: "Active",
      context: "W3-08 bounded partial ZIP metadata fixture"
    }),
    file({
      id: "w3-08-hostile",
      name: "archive-hostile.zip",
      path: "C:/Users/Zen/Documents/archive-hostile.zip",
      directory: "C:/Users/Zen/Documents",
      extension: "zip",
      size: 8_192,
      file_type: "Other",
      purpose: "Duplicate Review",
      lifecycle: "Active",
      context: "W3-08 hostile inert archive-name fixture"
    }),
    file({
      id: "w3-08-corrupt",
      name: "archive-corrupt.zip",
      path: "C:/Users/Zen/Documents/archive-corrupt.zip",
      directory: "C:/Users/Zen/Documents",
      extension: "zip",
      size: 128,
      file_type: "Other",
      purpose: "Duplicate Review",
      lifecycle: "Active",
      context: "W3-08 corrupt ZIP metadata fallback fixture"
    })
  );
}

if (isW201VirtualizationFixtureEnabled()) {
  mockFiles.push(...Array.from({ length: 125 }, (_, index) => {
    const sequence = String(index + 1).padStart(3, "0");
    return file({
      id: `w2-virtual-file-${sequence}`,
      name: `virtual-library-item-${sequence}.txt`,
      path: `C:/Users/Zen/Documents/virtual-library-item-${sequence}.txt`,
      directory: "C:/Users/Zen/Documents",
      extension: "txt",
      size: 4_096 + index,
      file_type: "Document",
      purpose: "Document",
      lifecycle: "Active",
      context: "W2-01 browser virtualization fixture"
    });
  }));
}

let mockLibraryRevision = 1;
let mockUserTags: UserTag[] = [
  {
    id: "mock-tag-work",
    displayName: "Work",
    colorToken: "blue",
    usageCount: 1,
    createdAt: Date.parse(now) / 1000,
    updatedAt: Date.parse(now) / 1000,
    revision: 1
  },
  {
    id: "mock-tag-review",
    displayName: "Review later",
    colorToken: "yellow",
    usageCount: 1,
    createdAt: Date.parse(now) / 1000,
    updatedAt: Date.parse(now) / 1000,
    revision: 1
  }
];
const mockFileTagIds = new Map<string, Set<string>>([
  ["mock-report", new Set(["mock-tag-work"])],
  ["mock-duplicate", new Set(["mock-tag-review"])]
]);
let mockLibrarySavedViews: LibrarySavedView[] = [];
let mockOrganizationPlans: OrganizationPlan[] = [];
const mockOrganizationItems = new Map<string, OrganizationPlanItem[]>();

const mockGlobalEntries: GlobalSearchResult[] = [
  {
    id: "global-mock-report",
    volumeId: "mock-volume",
    platformFileId: "path:C:/Users/Zen/Documents/project-report.pdf",
    name: "project-report.pdf",
    path: "C:/Users/Zen/Documents/project-report.pdf",
    extension: "pdf",
    isDirectory: false,
    size: 2_450_000,
    createdAtFs: null,
    modifiedAtFs: Date.parse(now) / 1000,
    fileAttributes: 0,
    isHidden: false,
    isSystem: false,
    sourceProvider: "windows_mft_usn",
    managed: false,
    rank: -1
  },
  {
    id: "global-mock-folder",
    volumeId: "mock-volume",
    platformFileId: "path:C:/Users/Zen/Documents",
    name: "Documents",
    path: "C:/Users/Zen/Documents",
    extension: "",
    isDirectory: true,
    size: 0,
    createdAtFs: null,
    modifiedAtFs: null,
    fileAttributes: 0,
    isHidden: false,
    isSystem: false,
    sourceProvider: "windows_mft_usn",
    managed: true,
    rank: -1
  },
  {
    id: "global-mock-archive",
    volumeId: "mock-volume",
    platformFileId: "path:C:/Users/Zen/Downloads/old-design-assets.zip",
    name: "old-design-assets.zip",
    path: "C:/Users/Zen/Downloads/old-design-assets.zip",
    extension: "zip",
    isDirectory: false,
    size: 84_000_000,
    createdAtFs: null,
    modifiedAtFs: Date.parse(now) / 1000,
    fileAttributes: 0,
    isHidden: false,
    isSystem: false,
    sourceProvider: "windows_mft_usn",
    managed: false,
    rank: -1
  }
];

let mockManagedScopeState: ManagedScope[] = [];
let mockManagedScanState: { request: ManagedScanRequest; start: ManagedScanStartDto } | null = null;
let mockDedupeRuns: DedupeRun[] = [];
let mockAnalysisRuns: AnalysisRun[] = [];
let mockAnalysisFindings: AnalysisFinding[] = [];

const mockAnalysisDetectors: AnalysisDetectorDescriptor[] = [
  {
    detectorId: "duplicate_reclaimable_v1",
    version: 1,
    title: "Duplicate reclaimable groups",
    description: "Read-only duplicate group findings.",
    supportsAllManagedScope: true,
    supportsApprovedPaths: false
  },
  {
    detectorId: "large_file_v1",
    version: 1,
    title: "Large files",
    description: "Large file review findings.",
    supportsAllManagedScope: true,
    supportsApprovedPaths: true
  },
  {
    detectorId: "large_directory_v1",
    version: 1,
    title: "Large directories",
    description: "Large directory review findings.",
    supportsAllManagedScope: true,
    supportsApprovedPaths: true
  },
  {
    detectorId: "cleanup_heuristics_v1",
    version: 1,
    title: "Cleanup heuristics",
    description: "Deterministic cleanup classification.",
    supportsAllManagedScope: true,
    supportsApprovedPaths: true
  }
];

const mockDuplicateGroups: DedupeGroup[] = [
  {
    id: "mock-duplicate-group",
    sizeEach: 810_000,
    fullHash: "mock-blake3-hash",
    fullHashAlgorithm: "blake3",
    fullHashVersion: 1,
    memberCount: 2,
    physicalCopyCount: 2,
    hardlinkAliasCount: 0,
    exactReclaimableBytes: 810_000,
    potentialReclaimableBytes: 810_000,
    reclaimableConfidence: "exact",
    status: "active",
    lastBuiltRunId: "mock-dedupe-run",
    revision: 1,
    createdAt: Math.floor(Date.now() / 1000),
    updatedAt: Math.floor(Date.now() / 1000),
    lastVerifiedAt: Math.floor(Date.now() / 1000),
    representativePaths: [
      "C:/Users/Zen/Desktop/invoice-copy.pdf",
      "C:/Users/Zen/Documents/invoice.pdf"
    ]
  }
];

const mockDuplicateMembers: DedupeGroupMember[] = [
  {
    groupId: "mock-duplicate-group",
    fileId: "mock-duplicate",
    pathSnapshot: "C:/Users/Zen/Desktop/invoice-copy.pdf",
    physicalKey: "windows:v1:mock:1",
    identityStatus: "verified",
    isHardlinkAlias: false,
    size: 810_000,
    modifiedNs: null,
    verifiedAt: Math.floor(Date.now() / 1000)
  },
  {
    groupId: "mock-duplicate-group",
    fileId: "mock-invoice",
    pathSnapshot: "C:/Users/Zen/Documents/invoice.pdf",
    physicalKey: "windows:v1:mock:2",
    identityStatus: "verified",
    isHardlinkAlias: false,
    size: 810_000,
    modifiedNs: null,
    verifiedAt: Math.floor(Date.now() / 1000)
  }
];

let mockCatalogRevision = 1;
let mockRules: Rule[] = [
  {
    id: "mock-user-rule-sensitive",
    name: "Mock sensitive review rule",
    source: "user",
    enabled: false,
    priority: 10,
    weight: 0.9,
    root_operator: "AND",
    groups: [{
      id: "mock-group-sensitive",
      operator: "AND",
      conditions: [{
        id: "mock-condition-sensitive",
        field: "risk_level",
        operator: "is",
        value: "Sensitive"
      }]
    }],
    action: { risk_level: "Sensitive", suggested_action: "Review" },
    created_at: now,
    updated_at: now,
    astVersion: 1,
    revision: 1,
    originProposalId: null
  }
];
let mockRuleProposals: RuleProposal[] = [];

const mockContentPolicy = (rootId: string, enabled = false): ContentScopePolicy => ({
  rootId,
  rootRevision: 1,
  enabled,
  extractorFamilies: ["txt", "md", "csv", "pdf_text", "docx", "xlsx", "pptx"],
  maxBytes: 8 * 1024 * 1024,
  maxChars: 32768,
  maxPages: 100,
  maxRows: 10000,
  rawRetentionMode: "none",
  rawRetentionChars: 0,
  localAllowed: true,
  cloudAllowed: false,
  policyRevision: enabled ? 1 : 0,
  updatedAt: Math.floor(Date.now() / 1000)
});

export async function mockInvokeCommand<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (isFileWorkspaceMockCommand(command)) {
    return mockFileWorkspaceInvoke<T>(command, args);
  }
  switch (command) {
    case "init_db":
    case "cancel_scan":
    case "cancel_operations":
    case "cancel_cleanup_restore":
    case "cancel_ai_classification":
    case "reveal_in_folder":
    case "open_global_search_result":
    case "reveal_global_search_result":
    case "reveal_storage_candidate":
    case "quit_app":
    case "insert_file":
    case "start_global_index":
    case "pause_global_index":
    case "resume_global_index":
    case "rebuild_global_index_source":
    case "set_global_index_source_enabled":
    case "mark_main_window_ready":
    case "acknowledge_main_window_ready":
    case "acknowledge_search_navigation":
      return undefined as T;
    case "activate_search_result":
      return mockActivateSearchResult(args?.request as Record<string, unknown> | undefined) as T;
    case "get_search_window_state":
      return mockSearchWindowState as T;
    case "search_window_ready":
      mockSearchWindowState = nextMockSearchWindowState("visible_collapsed");
      return mockSearchWindowState as T;
    case "resize_search_window": {
      const request = args?.request as { expanded?: boolean } | undefined;
      mockSearchWindowState = nextMockSearchWindowState(
        request?.expanded ? "visible_expanded" : "visible_collapsed"
      );
      return mockSearchWindowState as T;
    }
    case "hide_search_window_command":
      mockSearchWindowState = nextMockSearchWindowState("hidden");
      return mockSearchWindowState as T;
    case "start_dedupe_run":
      return startMockDedupeRun(args?.request as { parentScanSessionId?: string | null } | undefined) as T;
    case "retry_dedupe_run": {
      const previous = mockDedupeRuns.find((run) => run.id === String(args?.runId ?? ""));
      if (!previous) throw new Error("Mock dedupe run not found");
      return startMockDedupeRun({ parentScanSessionId: previous.parentScanSessionId }) as T;
    }
    case "cancel_dedupe_run": {
      const runId = String(args?.runId ?? "");
      const current = mockDedupeRuns.find((run) => run.id === runId);
      if (!current) throw new Error("Mock dedupe run not found");
      const cancelled = { ...current, status: "cancelled", cancelRequested: true, revision: current.revision + 1 };
      mockDedupeRuns = mockDedupeRuns.map((run) => run.id === runId ? cancelled : run);
      return cancelled as T;
    }
    case "cancel_dedupe": {
      const runId = String(args?.jobId ?? "");
      const current = mockDedupeRuns.find((run) => run.id === runId);
      if (!current) throw new Error("Mock dedupe run not found");
      const cancelled = { ...current, status: "cancelled", cancelRequested: true, revision: current.revision + 1 };
      mockDedupeRuns = mockDedupeRuns.map((run) => run.id === runId ? cancelled : run);
      return undefined as T;
    }
    case "get_dedupe_run": {
      const run = mockDedupeRuns.find((item) => item.id === String(args?.runId ?? ""));
      if (!run) throw new Error("Mock dedupe run not found");
      return run as T;
    }
    case "list_dedupe_runs":
      return mockDedupeRuns.slice(0, Number(args?.limit ?? 20)) as T;
    case "get_active_dedupe_run":
      return (mockDedupeRuns.find((run) => ["queued", "running", "cancelling"].includes(run.status)) ?? null) as T;
    case "list_analysis_detectors":
      return mockAnalysisDetectors as T;
    case "start_analysis_run":
      return startMockAnalysisRun(args?.request as StartAnalysisRunRequest | undefined) as T;
    case "retry_analysis_run": {
      const previous = mockAnalysisRuns.find((run) => run.id === String(args?.runId ?? ""));
      if (!previous) throw new Error("Mock analysis run not found");
      const paths = Array.isArray(previous.scope.paths) ? previous.scope.paths.map(String) : [];
      return startMockAnalysisRun({ requestKey: previous.requestKey, scope: { kind: "approvedCleanupPaths", paths } }) as T;
    }
    case "cancel_analysis_run": {
      const runId = String(args?.runId ?? "");
      const current = mockAnalysisRuns.find((run) => run.id === runId);
      if (!current) throw new Error("Mock analysis run not found");
      const cancelled = { ...current, status: "cancelled", cancelRequested: true, revision: current.revision + 1 };
      mockAnalysisRuns = mockAnalysisRuns.map((run) => run.id === runId ? cancelled : run);
      return cancelled as T;
    }
    case "get_analysis_run": {
      const run = mockAnalysisRuns.find((item) => item.id === String(args?.runId ?? ""));
      if (!run) throw new Error("Mock analysis run not found");
      return run as T;
    }
    case "get_active_analysis_run":
      return (mockAnalysisRuns.find((run) => ["queued", "running", "cancelling"].includes(run.status)) ?? null) as T;
    case "list_analysis_runs":
      return mockAnalysisRuns.slice(0, Number(args?.limit ?? 20)) as T;
    case "list_analysis_run_detectors":
      return [] as T;
    case "list_analysis_findings": {
      const findings = mockAnalysisFindings.filter((finding) => !args?.runId || finding.runId === String(args.runId));
      return { findings, nextCursor: null, limit: Number(args?.limit ?? 100) } satisfies AnalysisFindingPage as T;
    }
    case "get_analysis_finding":
      return (mockAnalysisFindings.find((finding) => finding.id === String(args?.findingId ?? "")) ?? null) as T;
    case "list_analysis_finding_evidence":
      return mockAnalysisFindingEvidence(String(args?.findingId ?? "")) as T;
    case "get_dedupe_authority":
      return { revision: 1, status: "healthy", lastAuthoritativeRunId: "mock-dedupe-run", scopeHash: "mock", updatedAt: Math.floor(Date.now() / 1000) } as T;
    case "set_analysis_finding_decision": {
      const decision: AnalysisFindingDecision = {
        findingKey: String(args?.findingKey ?? ""),
        decision: String(args?.decision ?? "open") as AnalysisFindingDecision["decision"],
        snoozedUntil: args?.snoozedUntil == null ? null : Number(args.snoozedUntil),
        note: args?.note == null ? null : String(args.note),
        revision: 1,
        createdAt: Math.floor(Date.now() / 1000),
        updatedAt: Math.floor(Date.now() / 1000)
      };
      if (mockCleanupPresentationScenario() === "request-unavailable") {
        mockAnalysisFindings = mockAnalysisFindings.map((finding) => finding.findingKey === decision.findingKey
          ? { ...finding, decision: decision.decision, decisionRevision: decision.revision }
          : finding);
      }
      return decision as T;
    }
    case "revalidate_analysis_finding":
      return (mockAnalysisFindings.find((finding) => finding.id === String(args?.findingId ?? "")) ?? null) as T;
    case "list_duplicate_groups":
      return {
        groups: mockDuplicateGroups,
        nextCursor: null,
        limit: Number(args?.limit ?? 50)
      } satisfies DedupeGroupPage as T;
    case "get_duplicate_group":
      return (mockDuplicateGroups.find((group) => group.id === String(args?.groupId ?? "")) ?? null) as T;
    case "list_duplicate_group_members":
      return mockDuplicateMembers.filter((member) => member.groupId === String(args?.groupId ?? "")) as T;
    case "get_file_duplicate_membership":
      return mockDuplicateGroups.filter((group) => mockDuplicateMembers.some((member) => member.fileId === String(args?.fileId ?? "") && member.groupId === group.id)) as T;
    case "start_managed_scan":
      return startMockManagedScan(args?.request as ManagedScanRequest | undefined) as T;
    case "get_managed_scan_snapshot": {
      const sessionId = String(args?.sessionId ?? "");
      const snapshot = mockManagedScanState?.start;
      if (!snapshot || snapshot.session.id !== sessionId) throw new Error("Mock scan session not found");
      return snapshot as ManagedScanSnapshotDto as T;
    }
    case "cancel_scan_run":
      return cancelMockManagedScan(String(args?.runId ?? "")) as T;
    case "retry_interrupted_scan": {
      const previous = mockManagedScanState?.start.runs.find((item) => item.id === String(args?.runId ?? ""));
      return startMockManagedScan({
        roots: [previous?.rootPath ?? "C:/Users/Zen"],
        requestKey: `mock-retry-${Date.now()}`,
        dedupe: false
      }) as T;
    }
    case "get_scan_run": {
      const run = mockManagedScanState?.start.runs.find((item) => item.id === String(args?.runId ?? ""));
      if (!run) throw new Error("Mock scan run not found");
      return run as T;
    }
    case "list_scan_runs":
      return (mockManagedScanState?.start.runs ?? []) as T;
    case "list_automation_intents":
    case "get_automation_intent":
    case "list_automation_runs":
    case "create_automation_intent":
    case "update_automation_intent":
    case "set_automation_intent_enabled":
    case "archive_automation_intent":
    case "run_automation_intent_manual":
      return automationMock(command, args) as T;
    case "list_scan_roots":
      return (mockManagedScanRoots()) as T;
    case "get_scan_root_health":
      return mockManagedRootsForRequest(mockManagedScanState?.request ?? { roots: [], dedupe: false })[0] as T;
    case "get_paged_files":
      return queryMockFiles(args) as T;
    case "query_file_library_v2": {
      const request = args?.request as FileQueryRequestV2 | undefined;
      if (isW211IntegratedFixtureEnabled()
        && request?.query.text?.trim().toLowerCase() === "slow-a") {
        await new Promise((resolve) => setTimeout(resolve, 500));
      }
      return queryMockFileLibraryV2(args) as T;
    }
    case "resolve_file_library_exact_count_v2":
      return {
        version: 2,
        requestId: String((args?.request as { requestId?: string } | undefined)?.requestId ?? ""),
        queryFingerprint: "",
        snapshotRevision: mockLibraryRevision,
        totalCount: isW211IntegratedFixtureEnabled()
          ? W211_LIBRARY_TOTAL
          : mockFiles.filter((file) => !file.is_stale).length,
        countState: "exact"
      } as T;
    case "get_file_library_detail":
      return getMockFileLibraryDetail(String(args?.fileId ?? "")) as T;
    case "get_file_library_selection_summary":
      return getMockFileLibrarySelectionSummary(args?.selection as LibrarySelectionV1 | undefined) as T;
    case "reveal_file_library_entry":
      throw new Error("browser_mock_reveal_unavailable");
    case "request_macos_thumbnail":
      throw new Error("browser_mock_quick_look_unavailable");
    case "cancel_macos_thumbnail":
      return false as T;
    case "list_user_tags":
      return mockUserTags as T;
    case "create_user_tag":
      return createMockUserTag(args?.request as CreateUserTagRequest | undefined) as T;
    case "update_user_tag":
      return updateMockUserTag(args?.request as UpdateUserTagRequest | undefined) as T;
    case "delete_user_tag":
      return deleteMockUserTag(args?.request as DeleteUserTagRequest | undefined) as T;
    case "mutate_file_user_tags":
      return mutateMockFileUserTags(args?.request as MutateFileUserTagsRequest | undefined) as T;
    case "list_library_saved_views":
      return mockLibrarySavedViews as T;
    case "create_library_saved_view":
      return createMockLibrarySavedView(args?.request as CreateLibrarySavedViewRequest | undefined) as T;
    case "update_library_saved_view":
      return updateMockLibrarySavedView(args?.request as UpdateLibrarySavedViewRequest | undefined) as T;
    case "delete_library_saved_view":
      return deleteMockLibrarySavedView(args?.request as DeleteLibrarySavedViewRequest | undefined) as T;
    case "create_organization_plan":
      return createMockOrganizationPlan(args?.request as { title?: string; source?: LibrarySelectionV1; expectedCount?: number } | undefined) as T;
    case "list_organization_plans":
      return listMockOrganizationPlans() as T;
    case "get_organization_plan": {
      const plan = mockOrganizationPlans.find((item) => item.id === String(args?.planId ?? ""));
      if (!plan) throw new Error("organization_plan_not_found");
      return plan as T;
    }
    case "query_organization_plan_items":
      return queryMockOrganizationItems(args?.request as { planId?: string; cursor?: string | null; pageSize?: number } | undefined) as T;
    case "query_organization_plan_groups":
      return queryMockOrganizationGroups(args?.request as { planId?: string; cursor?: string | null; pageSize?: number } | undefined) as T;
    case "query_organization_plan_group_items":
      return queryMockOrganizationGroupItems(args?.request as { planId?: string; groupId?: string; cursor?: string | null; expectedProjectionFingerprint?: string; pageSize?: number } | undefined) as T;
    case "update_organization_plan_decisions":
      return updateMockOrganizationDecisions(args?.request as MockOrganizationDecisionRequest | undefined) as T;
    case "update_organization_plan_group_decision":
      return updateMockOrganizationGroupDecision(args?.request as {
        planId?: string;
        groupId?: string;
        expectedPlanRevision?: number;
        expectedProjectionFingerprint?: string;
        expectedItemCount?: number;
        decision?: OrganizationPlanItem["decision"];
      } | undefined) as T;
    case "refresh_organization_plan":
      return refreshMockOrganizationPlan(args?.request as { planId?: string; expectedPlanRevision?: number } | undefined) as T;
    case "cancel_organization_plan":
      return cancelMockOrganizationPlan(args?.request as { planId?: string; expectedPlanRevision?: number } | undefined) as T;
    case "delete_organization_plan":
      return deleteMockOrganizationPlan(args?.request as { planId?: string; expectedPlanRevision?: number; confirmed?: boolean } | undefined) as T;
    case "analyze_organization_plan_items":
      return { planId: String((args?.request as { planId?: string } | undefined)?.planId ?? ""), queuedCount: 0, requiresRefresh: true } as T;
    case "get_organization_plan_dry_run":
      return getMockOrganizationDryRun(args?.request as { planId?: string; expectedPlanRevision?: number; itemIds?: string[]; allAccepted?: boolean } | undefined) as T;
    case "execute_organization_plan":
      throw new Error("browser_mock_native_execution_unavailable");
    case "get_stats_summary":
      return mockStats() as T;
    case "search_files":
      return searchMockFiles(String(args?.query ?? ""), Number(args?.limit ?? 12)) as T;
    case "search_global_entries":
      return searchMockGlobalEntries(args?.request as GlobalSearchRequest | undefined) as T;
    case "get_global_index_status":
      return mockGlobalIndexStatus() as T;
    case "list_global_index_sources":
      return mockGlobalIndexSources() as T;
    case "list_managed_scopes":
      return mockManagedScopeState as T;
    case "add_managed_scope":
      return addMockManagedScope(args?.request as AddManagedScopeRequest | undefined) as T;
    case "remove_managed_scope":
      mockManagedScopeState = mockManagedScopeState.filter((scope) => scope.id !== String(args?.id ?? ""));
      return true as T;
    case "update_managed_scope_policy":
      return updateMockManagedScope(args?.request as UpdateManagedScopePolicyRequest | undefined) as T;
    case "get_ai_management_status":
      return mockAiManagementStatus() as T;
    case "create_scan_job_id":
      return `scan-${args?.jobKind === "background" ? "background" : "foreground"}-${globalThis.crypto.randomUUID()}` as T;
    case "scan_directory":
      return {
        jobId: String(args?.jobId ?? "browser-mock-scan"),
        jobKind: args?.jobKind === "background" ? "background" : "foreground",
        root: String(args?.path ?? "C:/Users/Zen"),
        scanned: mockFiles.length,
        files: mockFiles.length,
        directories: 3,
        skipped: 0,
        errors: 0,
        elapsedMs: 1240
      } satisfies ScanSummary as T;
    case "execute_moves":
      return { logs: [], batch_id: "browser-mock-batch" } satisfies ExecuteOperationResult as T;
    case "get_manual_rename_operation_preview":
      throw new Error("file_mutation_unsupported_in_browser");
    case "get_permanent_delete_operation_preview":
      throw new Error("permanent_delete_unsupported");
    case "materialize_provider_preview":
      return {
        previewId: String((args?.request as Record<string, unknown> | undefined)?.previewId ?? "browser-mock-preview"),
        fileId: String((args?.request as Record<string, unknown> | undefined)?.fileId ?? "browser-mock-file"),
        materialization: "boundary_readable",
        nextOperationFingerprint: String((args?.request as Record<string, unknown> | undefined)?.operationFingerprint ?? "")
      } as T;
    case "restore_moves":
      return mockRestoreMoves(args) as T;
    case "resolve_operation_recovery":
      return mockResolveOperationRecovery(args) as T;
    case "get_operation_logs":
      return mockOperationLogs().slice(0, Number(args?.limit ?? 500)) as T;
    case "get_operation_previews_for_scope":
      return mockOperationPreviews(args) as T;
    case "get_operation_previews_by_file_ids": {
      const requested = new Set(Array.isArray(args?.fileIds) ? args.fileIds.map(String) : []);
      return mockOperationPreviews({ limit: 2000 }).previews.filter((preview) => requested.has(preview.fileId)) as T;
    }
    case "get_operation_previews_for_selection":
      return mockOperationPreviews(args) as T;
    case "start_storage_cleanup_scan":
      return "browser-mock-storage-cleanup-job" as T;
    case "get_storage_cleanup_scan_status":
      return mockStorageCleanupStatus(String(args?.jobId ?? "browser-mock-storage-cleanup-job")) as T;
    case "get_storage_cleanup_candidate_page":
      return mockStorageAnalysis() as T;
    case "cancel_storage_cleanup_scan":
      return undefined as T;
    case "move_cleanup_candidates_to_safe_trash":
      return mockSafeTrashExecutionResult(args) as T;
    case "analyze_cleanup_candidates_with_ai":
      return mockAnalyzeCleanupCandidatesWithAI(args) as T;
    case "list_cleanup_trash_batches":
      return mockCleanupTrashBatches() as T;
    case "preview_restore_cleanup_trash":
      {
        const batchId = String(args?.batchId ?? "browser-cleanup-batch");
        const batch = mockCleanupTrashBatches().find((item) => item.id === batchId);
        return {
          batchId,
          items: (batch?.items ?? []).map((item) => mockCleanupRestorePreviewItem(item))
        } satisfies CleanupRestorePreview as T;
      }
    case "restore_cleanup_trash_items":
      return mockCleanupRestoreResult(args) as T;
    case "preview_cleanup_candidates":
      return mockCleanupPreviewCandidates(args) as T;
    case "preview_cleanup_operations":
      return mockCleanupPreviewOperations(args) as T;
    case "execute_authoritative_rules_for_paths":
      return {
        scanned: Array.isArray(args?.paths) ? args.paths.length : 0,
        updated: 0,
        skipped: 0,
        needsConfirmation: 0
      } satisfies RuleExecutionSummary as T;
    case "execute_rules_for_scope_v2":
      return {
        summary: {
          scanned: mockFiles.length,
          updated: mockFiles.filter((item) => item.classification_status === "unclassified").length,
          skipped: 0,
          needsConfirmation: mockFiles.filter((item) => item.requires_confirmation).length
        },
        catalogRevision: mockCatalogRevision,
        classificationVersion: "browser-mock-not-native"
      } as T;
    case "get_content_scope_policy":
      return mockContentPolicy(String(args?.rootId ?? "mock-root")) as T;
    case "get_content_catalog_revision":
      throw new Error("browser_mock_content_unavailable: native content catalog requires desktop runtime");
    case "set_content_scope_policy":
      throw new Error("browser_mock_content_unavailable: native content policy persistence requires desktop runtime");
    case "preview_content":
    case "start_content_run":
    case "get_content_run":
    case "list_content_runs":
    case "get_active_content_run_for_file":
    case "cancel_content_run":
    case "query_content_run_items":
    case "get_content_artifact":
    case "query_content_artifacts":
    case "rebuild_content_artifact":
    case "delete_content_artifact":
    case "purge_content_scope":
    case "understand_content_artifacts":
      throw new Error("browser_mock_content_unavailable: native content extraction/provider/persistence is unavailable in the browser mock");
    case "classify_files_with_ai":
      return mockAIClassifyFiles(args) as T;
    case "classify_selected_files_with_ai":
      return mockAIClassifySelectedFiles(args) as T;
    case "confirm_classification":
      return undefined as T;
    case "correct_classification":
      return mockCorrectClassification(args) as T;
    case "list_user_rules_v2":
      return mockRules as T;
    case "get_rule_catalog_state":
      return { revision: mockCatalogRevision, updatedAt: Math.floor(Date.now() / 1000) } as T;
    case "create_user_rule_v2":
      return createMockUserRule(args?.request as { draft?: RuleDraftV2 } | undefined) as T;
    case "update_user_rule_v2":
      return updateMockUserRule(args?.request as {
        ruleId?: string;
        expectedRuleRevision?: number;
        expectedCatalogRevision?: number;
        draft?: RuleDraftV2;
      } | undefined) as T;
    case "set_user_rule_enabled_v2":
      return toggleMockUserRule(args?.request as {
        ruleId?: string;
        expectedRuleRevision?: number;
        expectedCatalogRevision?: number;
        enabled?: boolean;
      } | undefined) as T;
    case "delete_user_rule_v2":
      return deleteMockUserRule(args?.request as {
        ruleId?: string;
        expectedRuleRevision?: number;
        expectedCatalogRevision?: number;
        confirmed?: boolean;
      } | undefined) as T;
    case "create_rule_proposal":
      return createMockRuleProposal(args?.request as Record<string, unknown> | undefined) as T;
    case "regenerate_rule_proposal":
      return regenerateMockRuleProposal(args?.request as Record<string, unknown> | undefined) as T;
    case "get_rule_proposal":
      return requireMockProposal(String(args?.proposalId ?? "")) as T;
    case "list_rule_proposals":
      return {
        proposals: [...mockRuleProposals].sort((left, right) => right.updatedAt - left.updatedAt),
        nextCursor: null,
        hasMore: false
      } as T;
    case "cancel_rule_proposal":
      return cancelMockRuleProposal(args?.request as Record<string, unknown> | undefined) as T;
    case "delete_rule_proposal":
      return deleteMockRuleProposal(args?.request as Record<string, unknown> | undefined) as T;
    case "replace_rule_proposal_candidate":
      return replaceMockRuleProposalCandidate(args?.request as Record<string, unknown> | undefined) as T;
    case "preview_rule_proposal":
      return previewMockRuleProposal(args?.request as Record<string, unknown> | undefined) as T;
    case "resolve_rule_proposal_exact_impact":
      return previewMockRuleProposal(args?.request as Record<string, unknown> | undefined) as T;
    case "apply_rule_proposal":
      return applyMockRuleProposal(args?.request as Record<string, unknown> | undefined) as T;
    case "get_settings":
      return getMockVersionedSettings() as T;
    case "save_settings":
      return saveMockVersionedSettings(args?.request as SaveSettingsRequest) as T;
    case "get_ai_settings":
      return mockAISettings() as T;
    case "get_ai_feature_readiness":
      return browserPresentationFeatureReadinessFixture() as T;
    case "get_runtime_capabilities":
      return {
        platform: "browser",
        architecture: "unknown",
        macosVersion: null,
        aiDebugAvailable: true,
        realAIClassificationAvailable: true,
        credentialStoreAvailable: true,
        fileMutationAvailable: true,
        fileMutationUnavailableCode: null,
        copyAvailable: true,
        duplicateAvailable: true,
        renameAvailable: true,
        sameVolumeMoveAvailable: true,
        crossVolumeMoveAvailable: false,
        replaceAvailable: true,
        safeTrashAvailable: true,
        restoreAvailable: true,
        permanentDeleteAvailable: false,
        secureRemovalAvailable: false,
        packageMutationAvailable: true,
        iCloudMutationAvailable: true,
        fileProviderMutationAvailable: false,
        externalVolumeMutationAvailable: false,
        networkVolumeMutationAvailable: false,
        backendWatcherReconciliation: true,
        macosNativeSemanticsAvailable: false,
        macosSameVolumeMutationAvailable: false,
        macosRenameAvailable: false,
        macosSafeTrashAvailable: false,
        macosCloudMutationAvailable: false,
        macosFileProviderMutationAvailable: false,
        macosPackageMutationAvailable: false,
        macosCrossVolumeMutationAvailable: false,
        macosLifecycleAvailable: false,
        macosFinderAvailable: false,
        macosQuickLookThumbnailAvailable: false,
        macosQuickLookPreviewAvailable: false,
        macosRestoreAvailable: false,
        macosActivityPolicyAvailable: false,
        macosICloudAwarenessAvailable: false,
        macosFileProviderAwarenessAvailable: false,
        macosPackageAwarenessAvailable: false,
        fileProviderCapabilities: {
          platformFeatureAvailability: "unavailable",
          runtimeEnvironmentCapability: "unavailable",
          operationEligibility: "unavailable"
        },
        externalVolumeCapabilities: {
          platformFeatureAvailability: "unavailable",
          runtimeEnvironmentCapability: "runtimeDependent",
          operationEligibility: "notFixtureValidated"
        },
        networkVolumeCapabilities: {
          platformFeatureAvailability: "unavailable",
          runtimeEnvironmentCapability: "runtimeDependent",
          operationEligibility: "notFixtureValidated"
        }
      } as T;
    case "save_ai_settings":
      return mockAISettings(args?.settings as AISettings | undefined) as T;
    case "list_ai_provider_presets":
      return mockAIProviderPresets() as T;
    case "list_ai_models":
      return mockAIModels() as T;
    case "test_ai_provider_connection":
      return mockAIConnectionTest(args?.settings as AISettings | undefined) as T;
    case "list_ai_request_traces":
      return mockAITraces() as T;
    case "clear_ai_request_traces":
      mockAITraceState = [];
      return undefined as T;
    case "export_ai_request_traces":
      return JSON.stringify(mockAITraces(), null, 2) as T;
    case "debug_ai_classification_once":
      return mockAIDebugClassification(args) as T;
    case "get_global_hotkey_status":
    case "register_global_search_hotkey":
      return {
        requestedAccelerator: String(args?.accelerator ?? DEFAULT_SEARCH_HOTKEY),
        effectiveAccelerator: null,
        registered: false,
        error: "Browser mock mode",
        revision: 1
      } satisfies GlobalHotkeyStatus as T;
    case "remove_files_by_paths":
    case "upsert_files_by_paths":
      return 0 as T;
    default:
      throw new Error(`Unsupported mock command: ${command}`);
  }
}

function mockRuleFromDraft(
  draft: RuleDraftV2,
  id: string,
  revision: number,
  originProposalId: string | null
): Rule {
  return {
    id,
    name: draft.name.trim(),
    source: "user",
    enabled: false,
    priority: draft.priority,
    weight: draft.weight,
    root_operator: draft.rootOperator,
    groups: draft.groups.map((group, groupIndex) => ({
      id: `mock-group-${groupIndex + 1}`,
      operator: group.operator,
      conditions: group.conditions.map((condition, conditionIndex) => ({
        id: `mock-condition-${groupIndex + 1}-${conditionIndex + 1}`,
        field: condition.field,
        operator: condition.operator,
        value: condition.value
      }))
    })),
    action: {
      purpose: draft.action.purpose,
      lifecycle: draft.action.lifecycle,
      context: draft.action.context,
      risk_level: draft.action.riskLevel,
      suggested_action: draft.action.suggestedAction,
      target_template: draft.action.targetTemplate,
      rename_template: draft.action.renameTemplate
    },
    created_at: now,
    updated_at: new Date().toISOString(),
    astVersion: 1,
    revision,
    originProposalId
  };
}

function requireMockCatalog(expected: unknown) {
  if (Number(expected) !== mockCatalogRevision) throw new Error("rule_catalog_revision_conflict");
}

function requireMockRule(id: string, expectedRevision?: unknown) {
  const rule = mockRules.find((candidate) => candidate.id === id);
  if (!rule) throw new Error("rule_not_found");
  if (expectedRevision !== undefined && rule.revision !== Number(expectedRevision)) {
    throw new Error("rule_revision_conflict");
  }
  return rule;
}

function createMockUserRule(request?: { draft?: RuleDraftV2 }) {
  if (!request?.draft) throw new Error("rule_create_request_invalid");
  const rule = mockRuleFromDraft(
    request.draft,
    `mock-user-rule-${globalThis.crypto.randomUUID()}`,
    1,
    null
  );
  mockRules = [...mockRules, rule];
  mockCatalogRevision += 1;
  return { rule, catalogRevision: mockCatalogRevision };
}

function updateMockUserRule(request?: {
  ruleId?: string;
  expectedRuleRevision?: number;
  expectedCatalogRevision?: number;
  draft?: RuleDraftV2;
}) {
  if (!request?.draft || !request.ruleId) throw new Error("rule_update_request_invalid");
  requireMockCatalog(request.expectedCatalogRevision);
  const current = requireMockRule(request.ruleId, request.expectedRuleRevision);
  const rule = {
    ...mockRuleFromDraft(request.draft, current.id, (current.revision ?? 0) + 1, current.originProposalId ?? null),
    enabled: current.enabled,
    created_at: current.created_at
  };
  mockRules = mockRules.map((candidate) => candidate.id === rule.id ? rule : candidate);
  mockCatalogRevision += 1;
  return { rule, catalogRevision: mockCatalogRevision };
}

function toggleMockUserRule(request?: {
  ruleId?: string;
  expectedRuleRevision?: number;
  expectedCatalogRevision?: number;
  enabled?: boolean;
}) {
  if (!request?.ruleId || typeof request.enabled !== "boolean") throw new Error("rule_toggle_request_invalid");
  requireMockCatalog(request.expectedCatalogRevision);
  const current = requireMockRule(request.ruleId, request.expectedRuleRevision);
  const rule = {
    ...current,
    enabled: request.enabled,
    revision: (current.revision ?? 0) + 1,
    updated_at: new Date().toISOString()
  };
  mockRules = mockRules.map((candidate) => candidate.id === rule.id ? rule : candidate);
  mockCatalogRevision += 1;
  return { rule, catalogRevision: mockCatalogRevision };
}

function deleteMockUserRule(request?: {
  ruleId?: string;
  expectedRuleRevision?: number;
  expectedCatalogRevision?: number;
  confirmed?: boolean;
}) {
  if (!request?.confirmed || !request.ruleId) throw new Error("rule_delete_confirmation_required");
  requireMockCatalog(request.expectedCatalogRevision);
  requireMockRule(request.ruleId, request.expectedRuleRevision);
  mockRules = mockRules.filter((rule) => rule.id !== request.ruleId);
  mockCatalogRevision += 1;
  return { revision: mockCatalogRevision, updatedAt: Math.floor(Date.now() / 1000) };
}

function mockDraftForPrompt(prompt: string): RuleDraftV2 {
  const extension = prompt.match(/\b(pdf|png|jpg|jpeg|zip|docx|xlsx)\b/i)?.[1]?.toLowerCase();
  const firstLiteral = prompt.trim().split(/\s+/)[0] || "file";
  return {
    name: `MOCK proposal: ${prompt.trim().slice(0, 64)}`,
    priority: 75,
    weight: 75,
    rootOperator: "AND",
    groups: [{
      operator: "AND",
      conditions: [{
        field: extension ? "extension" : "name",
        operator: extension ? "equals" : "contains",
        value: extension ?? firstLiteral
      }]
    }],
    action: {
      purpose: "Work",
      lifecycle: "Inbox",
      suggestedAction: "Review"
    }
  };
}

function proposalCandidateFromDraft(draft: RuleDraftV2): NonNullable<RuleProposal["candidate"]> {
  const rule = mockRuleFromDraft(draft, "mock-candidate", 1, null);
  return {
    astVersion: 1,
    name: rule.name,
    priority: rule.priority,
    weight: rule.weight,
    rootOperator: rule.root_operator === "OR" ? "OR" : "AND",
    groups: rule.groups,
    action: rule.action
  };
}

function createMockRuleProposal(request?: Record<string, unknown>): RuleProposal {
  const prompt = String(request?.prompt ?? "").trim();
  if (!prompt) throw new Error("rule_proposal_prompt_invalid");
  const id = String(request?.proposalId ?? `rule-proposal-${globalThis.crypto.randomUUID()}`);
  const draft = mockDraftForPrompt(prompt);
  const timestamp = Math.floor(Date.now() / 1000);
  const proposal: RuleProposal = {
    id,
    status: "ready",
    intentKind: request?.intentKind === "update" ? "update" : "create",
    targetRuleId: typeof request?.targetRuleId === "string" ? request.targetRuleId : null,
    baseRuleRevision: typeof request?.expectedTargetRuleRevision === "number"
      ? request.expectedTargetRuleRevision
      : null,
    prompt,
    promptFingerprint: `mock-prompt-${prompt.length}`,
    providerKind: "openai_compatible",
    providerPreset: "deepseek",
    model: "browser-mock-deterministic",
    candidateOrigin: "provider",
    astVersion: 1,
    candidate: proposalCandidateFromDraft(draft),
    candidateFingerprint: `mock-candidate-${prompt.length}`,
    summary: "MOCK deterministic proposal; no real AI request or native persistence occurred.",
    clarifications: [],
    validation: {
      valid: true,
      permissionClass: "allow",
      requiresConfirmation: false,
      broadMatch: false,
      codes: ["browser_mock_only"],
      warnings: ["browser_mock_not_native_persistence"]
    },
    appliedRuleId: null,
    revision: 3,
    lastErrorCode: null,
    lastErrorDetail: null,
    createdAt: timestamp,
    updatedAt: timestamp,
    generatedAt: timestamp,
    appliedAt: null
  };
  mockRuleProposals = [proposal, ...mockRuleProposals.filter((item) => item.id !== id)];
  return proposal;
}

function requireMockProposal(id: string, expectedRevision?: unknown) {
  const proposal = mockRuleProposals.find((candidate) => candidate.id === id);
  if (!proposal) throw new Error("rule_proposal_not_found");
  if (expectedRevision !== undefined && proposal.revision !== Number(expectedRevision)) {
    throw new Error("rule_proposal_revision_conflict");
  }
  return proposal;
}

function regenerateMockRuleProposal(request?: Record<string, unknown>) {
  const id = String(request?.proposalId ?? "");
  const current = requireMockProposal(id, request?.expectedProposalRevision);
  const generated = createMockRuleProposal({ ...request, proposalId: id });
  const proposal = {
    ...generated,
    createdAt: current.createdAt,
    revision: current.revision + 2
  };
  mockRuleProposals = [proposal, ...mockRuleProposals.filter((item) => item.id !== id)];
  return proposal;
}

function cancelMockRuleProposal(request?: Record<string, unknown>) {
  const id = String(request?.proposalId ?? "");
  const current = requireMockProposal(id);
  const proposal: RuleProposal = {
    ...current,
    status: "cancelled",
    revision: current.revision + 1,
    updatedAt: Math.floor(Date.now() / 1000)
  };
  mockRuleProposals = [proposal, ...mockRuleProposals.filter((item) => item.id !== id)];
  return proposal;
}

function deleteMockRuleProposal(request?: Record<string, unknown>) {
  const id = String(request?.proposalId ?? "");
  const current = requireMockProposal(id, request?.expectedProposalRevision);
  if (!request?.confirmed || !["applied", "cancelled", "invalid", "failed"].includes(current.status)) {
    throw new Error("rule_proposal_delete_blocked");
  }
  mockRuleProposals = mockRuleProposals.filter((proposal) => proposal.id !== id);
  return true;
}

function replaceMockRuleProposalCandidate(request?: Record<string, unknown>) {
  const id = String(request?.proposalId ?? "");
  const current = requireMockProposal(id, request?.expectedProposalRevision);
  const draft = request?.candidate as RuleDraftV2 | undefined;
  if (!draft) throw new Error("rule_proposal_candidate_invalid");
  const proposal: RuleProposal = {
    ...current,
    status: "ready",
    candidate: proposalCandidateFromDraft(draft),
    candidateFingerprint: `mock-edited-${current.revision + 1}`,
    revision: current.revision + 1,
    updatedAt: Math.floor(Date.now() / 1000)
  };
  mockRuleProposals = [proposal, ...mockRuleProposals.filter((item) => item.id !== id)];
  return proposal;
}

function previewMockRuleProposal(request?: Record<string, unknown>): RuleProposalImpact {
  const proposal = requireMockProposal(
    String(request?.proposalId ?? ""),
    request?.expectedProposalRevision
  );
  const extension = proposal.candidate?.groups[0]?.conditions[0]?.field === "extension"
    ? String(proposal.candidate.groups[0].conditions[0].value)
    : null;
  const matched = extension
    ? mockFiles.filter((file) => file.extension.toLowerCase() === extension.toLowerCase())
    : mockFiles;
  return {
    proposalId: proposal.id,
    proposalRevision: proposal.revision,
    candidateFingerprint: proposal.candidateFingerprint ?? "mock-candidate",
    catalogRevision: mockCatalogRevision,
    libraryRevision: 1,
    scopeHealth: { state: "healthy", roots: [], invalidReferences: [], message: null },
    permissionClass: proposal.validation.permissionClass,
    impactState: "exact",
    matchedCount: matched.length,
    impactToken: null,
    sampleRows: matched.slice(0, 20).map((file) => ({
      fileId: file.id,
      name: file.name,
      extension: file.extension,
      size: file.size,
      modifiedAt: 0,
      fileType: file.file_type,
      riskLevel: file.risk_level,
      beforeAction: file.suggested_action,
      afterAction: proposal.candidate?.action.suggested_action ?? null
    })),
    sampleIsBounded: true,
    actionSummary: proposal.candidate?.action ?? {},
    riskSummary: ["browser_mock_only"],
    requiresConfirmation: true,
    broadMatch: false,
    conflictAnalysisState: "browser_mock_bounded",
    conflicts: [],
    previewFingerprint: `mock-preview-${proposal.id}-${proposal.revision}-${mockCatalogRevision}`
  };
}

function applyMockRuleProposal(request?: Record<string, unknown>) {
  const proposal = requireMockProposal(
    String(request?.proposalId ?? ""),
    request?.expectedProposalRevision
  );
  requireMockCatalog(request?.expectedCatalogRevision);
  if (!request?.confirmed || proposal.status !== "ready" || !proposal.candidate) {
    throw new Error("rule_proposal_apply_blocked");
  }
  const draft: RuleDraftV2 = {
    name: proposal.candidate.name,
    priority: proposal.candidate.priority,
    weight: proposal.candidate.weight,
    rootOperator: proposal.candidate.rootOperator,
    groups: proposal.candidate.groups.map((group) => ({
      operator: group.operator === "OR" ? "OR" : "AND",
      conditions: group.conditions.map((condition) => ({
        field: condition.field as Exclude<typeof condition.field, "unknown">,
        operator: condition.operator as Exclude<typeof condition.operator, "unknown">,
        value: condition.value
      }))
    })),
    action: {
      purpose: proposal.candidate.action.purpose,
      lifecycle: proposal.candidate.action.lifecycle,
      context: proposal.candidate.action.context,
      riskLevel: proposal.candidate.action.risk_level,
      suggestedAction: proposal.candidate.action.suggested_action,
      targetTemplate: proposal.candidate.action.target_template,
      renameTemplate: proposal.candidate.action.rename_template
    }
  };
  let rule: Rule;
  if (proposal.intentKind === "update" && proposal.targetRuleId) {
    const current = requireMockRule(proposal.targetRuleId, request?.expectedTargetRuleRevision);
    rule = {
      ...mockRuleFromDraft(draft, current.id, (current.revision ?? 0) + 1, proposal.id),
      enabled: current.enabled,
      created_at: current.created_at
    };
    mockRules = mockRules.map((candidate) => candidate.id === rule.id ? rule : candidate);
  } else {
    rule = mockRuleFromDraft(draft, `mock-user-rule-${globalThis.crypto.randomUUID()}`, 1, proposal.id);
    mockRules = [...mockRules, rule];
  }
  mockCatalogRevision += 1;
  const applied: RuleProposal = {
    ...proposal,
    status: "applied",
    appliedRuleId: rule.id,
    revision: proposal.revision + 1,
    appliedAt: Math.floor(Date.now() / 1000),
    updatedAt: Math.floor(Date.now() / 1000)
  };
  mockRuleProposals = [applied, ...mockRuleProposals.filter((item) => item.id !== applied.id)];
  return { proposal: applied, rule, catalogRevision: mockCatalogRevision };
}

function startMockDedupeRun(request?: { parentScanSessionId?: string | null }): DedupeRun {
  const now = Math.floor(Date.now() / 1000);
  const run: DedupeRun = {
    id: `mock-dedupe-run-${Date.now()}`,
    requestKey: `mock-dedupe-request-${Date.now()}`,
    requestAttempt: 1,
    parentScanSessionId: request?.parentScanSessionId ?? null,
    scope: { kind: "all_managed_file_library", rootIds: [] },
    scopeSnapshot: [],
    scopeHash: "mock-scope-hash",
    scopeSnapshotHash: "mock-snapshot-hash",
    publicationMode: "authoritative",
    status: "completed",
    phase: "completed",
    revision: 2,
    cancelRequested: false,
    rerunRequired: false,
    candidateFiles: mockFiles.length,
    candidatePhysicalObjects: mockFiles.length,
    candidateBytes: mockFiles.reduce((total, file) => total + file.size, 0),
    identityVerifiedFiles: mockFiles.length,
    identityUnknownFiles: 0,
    hardlinkAliases: 0,
    prehashedFiles: mockFiles.length,
    prehashPrunedFiles: 0,
    fullHashedFiles: mockFiles.length,
    duplicateGroups: 1,
    duplicateMembers: 2,
    exactReclaimableBytes: 810_000,
    potentialReclaimableBytes: 810_000,
    processedFiles: mockFiles.length,
    processedBytes: mockFiles.reduce((total, file) => total + file.size, 0),
    totalBytes: mockFiles.reduce((total, file) => total + file.size, 0),
    warningCount: 0,
    errorCount: 0,
    startedAt: now,
    finishedAt: now,
    lastCheckpointAt: now,
    createdAt: now,
    updatedAt: now,
    errorCode: null,
    errorMessage: null
  };
  mockDedupeRuns = [run, ...mockDedupeRuns].slice(0, 20);
  return run;
}

function startMockAnalysisRun(request?: StartAnalysisRunRequest): AnalysisRun {
  const now = Math.floor(Date.now() / 1000);
  const scenario = mockCleanupPresentationScenario();
  const failed = scenario === "analysis-failure";
  const hasPresentationFindings = scenario === "current-assessment" || scenario === "request-unavailable" || scenario === "preview";
  const requestedPaths = request?.scope?.paths?.map((path) => path.trim()).filter(Boolean) ?? [];
  const cleanupCandidates = mockStorageAnalysis().candidates;
  const safeCandidate = cleanupCandidates.find((candidate) => candidate.id === "storage-safe-node-modules");
  const reviewCandidate = cleanupCandidates.find((candidate) => candidate.id === "storage-review-download");
  const rootPath = requestedPaths[0] ?? "C:/Users/Zen/Documents";
  const safeFindingId = safeCandidate?.id ?? "browser-presentation-cleanup-safe";
  const reviewFindingId = reviewCandidate?.id ?? "browser-presentation-cleanup-review";
  const safePath = `${rootPath.replace(/[\\/]$/, "")}/node_modules`;
  const reviewPath = `${rootPath.replace(/[\\/]$/, "")}/course-video.mp4`;
  const run: AnalysisRun = {
    id: `mock-analysis-run-${Date.now()}`,
    requestKey: request?.requestKey ?? `mock-analysis-request-${Date.now()}`,
    requestAttempt: 1,
    scope: { kind: "approved_cleanup_paths", paths: requestedPaths },
    scopeHash: "mock-analysis-scope",
    sourceSnapshot: {},
    sourceSnapshotHash: "mock-analysis-snapshot",
    detectorSet: mockAnalysisDetectors.map((detector) => `${detector.detectorId}:v${detector.version}`),
    detectorSetHash: "mock-analysis-detectors",
    status: failed ? "failed" : "completed",
    phase: failed ? "failed" : "completed",
    revision: 2,
    cancelRequested: false,
    rerunRequired: false,
    detectorsTotal: mockAnalysisDetectors.length,
    detectorsCompleted: mockAnalysisDetectors.length,
    detectorsFailed: failed ? 1 : 0,
    findingsStaged: hasPresentationFindings ? 2 : 0,
    findingsPublished: hasPresentationFindings ? 2 : 0,
    safeCount: hasPresentationFindings ? 1 : 0,
    reviewCount: hasPresentationFindings ? 1 : 0,
    cautionCount: 0,
    exactReclaimableBytes: hasPresentationFindings ? safeCandidate?.size ?? 1024 : 0,
    potentialReclaimableBytes: hasPresentationFindings ? (safeCandidate?.size ?? 1024) + (reviewCandidate?.size ?? 1024) : 0,
    warningCount: 0,
    errorCount: failed ? 1 : 0,
    startedAt: now,
    finishedAt: now,
    lastCheckpointAt: now,
    createdAt: now,
    updatedAt: now,
    errorCode: failed ? "browser_presentation_analysis_failure" : null,
    errorMessage: failed ? "Presentation-only browser fixture: cleanup analysis failed." : null
  };
  mockAnalysisFindings = hasPresentationFindings
    ? [
      makeMockCleanupFinding(run, safeCandidate, safeFindingId, safePath, "safe"),
      makeMockCleanupFinding(run, reviewCandidate, reviewFindingId, reviewPath, "review")
    ]
    : [];
  mockAnalysisRuns = [run, ...mockAnalysisRuns].slice(0, 20);
  return run;
}

function mockCleanupPresentationScenario(): "none" | "analysis-failure" | "current-assessment" | "request-unavailable" | "preview" {
  const value = new URLSearchParams(globalThis.location?.search ?? "").get("pm01-cleanup");
  return value === "analysis-failure" || value === "current-assessment" || value === "request-unavailable" || value === "preview" ? value : "none";
}

function makeMockCleanupFinding(
  run: AnalysisRun,
  candidate: StorageAnalysis["candidates"][number] | undefined,
  id: string,
  pathSnapshot: string,
  tier: "safe" | "review"
): AnalysisFinding {
  const now = Math.floor(Date.now() / 1000);
  return {
    id,
    findingKey: `browser-presentation-only:${id}`,
    runId: run.id,
    detectorId: "cleanup_heuristics_v1",
    detectorVersion: 1,
    scopeHash: run.scopeHash,
    status: "active",
    tier,
    category: candidate?.category ?? "Browser presentation fixture",
    actionKind: "safe_trash_candidate",
    title: candidate?.name ?? (tier === "safe" ? "node_modules" : "course-video.mp4"),
    reason: candidate?.reason ?? "Browser presentation fixture only.",
    riskNote: candidate?.risk_note ?? null,
    confidence: tier === "safe" ? "exact" : "estimated",
    sizeBytes: candidate?.size ?? 1024,
    exactReclaimableBytes: tier === "safe" ? (candidate?.size ?? 1024) : null,
    potentialReclaimableBytes: candidate?.size ?? 1024,
    requiresConfirmation: tier === "review",
    executable: true,
    primarySubjectKind: "approved_path",
    primarySubjectId: pathSnapshot,
    pathSnapshot,
    identitySnapshot: { presentationFixture: true },
    evidenceSummary: { presentationOnly: true },
    revision: 1,
    createdAt: now,
    updatedAt: now,
    publishedAt: now,
    staleAt: null,
    decision: null,
    snoozedUntil: null,
    decisionRevision: null
  };
}

function mockAnalysisFindingEvidence(findingId: string): AnalysisFindingEvidence[] {
  if (!["current-assessment", "request-unavailable"].includes(mockCleanupPresentationScenario()) || !mockAnalysisFindings.some((finding) => finding.id === findingId && finding.tier === "review")) return [];
  return [{
    id: `browser-presentation-only-assessment:${findingId}`,
    findingId,
    evidenceKind: "ai_assessment",
    subjectKind: "analysis_finding",
    subjectId: findingId,
    pathSnapshot: mockAnalysisFindings.find((finding) => finding.id === findingId)?.pathSnapshot ?? null,
    value: { schema: "cleanup_ai_assessment_v1", presentationOnly: true },
    createdAt: Math.floor(Date.now() / 1000),
    isCurrentAssessment: true
  }];
}

function startMockManagedScan(request?: ManagedScanRequest): ManagedScanStartDto {
  const normalizedRequest: ManagedScanRequest = {
    roots: request?.roots?.map((root) => root.trim()).filter(Boolean) ?? [],
    requestKey: request?.requestKey ?? null,
    dedupe: request?.dedupe ?? false
  };
  if (
    mockManagedScanState
    && normalizedRequest.requestKey
    && mockManagedScanState.request.requestKey === normalizedRequest.requestKey
  ) {
    return mockManagedScanState.start;
  }
  const now = Math.floor(Date.now() / 1000);
  const sessionId = `mock-scan-session-${Date.now()}`;
  const roots = normalizedRequest.roots.map((root, index) => ({
    sessionId,
    requestedIndex: index,
    requestedPath: root,
    normalizedRequestedPath: root.replaceAll("\\\\", "/"),
    resolution: "effective",
    effectiveRootId: `mock-scan-root-${index}`,
    effectivePath: root,
    effectiveIndex: index,
    runId: `mock-scan-run-${Date.now()}-${index}`,
    status: "completed",
    reason: null,
    createdAt: now,
    updatedAt: now
  }));
  const runs: ScanRunDto[] = roots.map((root) => ({
    id: root.runId ?? "",
    scanRootId: root.effectiveRootId ?? "",
    rootPath: root.effectivePath ?? root.requestedPath,
    generation: 1,
    parentSessionId: sessionId,
    status: "completed",
    phase: "completed",
    scannedFiles: mockFiles.length,
    scannedDirectories: 3,
    processedBytes: mockFiles.reduce((total, file) => total + file.size, 0),
    warningsCount: 0,
    errorsCount: 0,
    metadataErrorCount: 0,
    coverageErrorCount: 0,
    coverageComplete: true,
    staleReconciliationAllowed: false,
    cancelRequested: false,
    revision: 4,
    sessionRevision: 4,
    startedAt: now - 1,
    finishedAt: now,
    lastCheckpointAt: now,
    errorCode: null,
    errorMessage: null,
    resultJson: null,
    watcherRevisionAtStart: 1,
    createdAt: now - 1,
    updatedAt: now
  }));
  const session: ScanSessionDto = {
    id: sessionId,
    requestKey: normalizedRequest.requestKey ?? null,
    canonicalRequestHash: "browser-mock-canonical-request",
    status: "completed",
    phase: "completed",
    cancelRequested: false,
    requestedRootCount: roots.length,
    effectiveRootCount: roots.length,
    completedRootCount: roots.length,
    failedRootCount: 0,
    cancelledRootCount: 0,
    coveredRootCount: 0,
    unstartedRootCount: 0,
    dedupeRequested: normalizedRequest.dedupe,
    dedupeDispatchState: normalizedRequest.dedupe ? "pending" : "not_requested",
    dedupeAttemptCount: 0,
    dedupeJobId: null,
    dedupeLastError: null,
    scannedFiles: runs.reduce((total, run) => total + run.scannedFiles, 0),
    scannedDirectories: runs.reduce((total, run) => total + run.scannedDirectories, 0),
    warningsCount: 0,
    errorsCount: 0,
    revision: 4,
    startedAt: now - 1,
    finishedAt: now,
    lastCheckpointAt: now,
    errorCode: null,
    errorMessage: null,
    resultJson: null,
    createdAt: now - 1,
    updatedAt: now,
    roots
  };
  const start = { session, runs };
  mockManagedScanState = { request: normalizedRequest, start };
  return start;
}

function cancelMockManagedScan(runId: string): ScanRunDto {
  const current = mockManagedScanState?.start.runs.find((run) => run.id === runId);
  if (!current) throw new Error("Mock scan run not found");
  const run: ScanRunDto = { ...current, status: "cancelled", cancelRequested: true, errorCode: "cancelled" };
  const start = mockManagedScanState!.start;
  mockManagedScanState = {
    request: mockManagedScanState!.request,
    start: {
      ...start,
      runs: start.runs.map((item) => item.id === runId ? run : item),
      session: { ...start.session, status: "cancelled", cancelRequested: true }
    }
  };
  return run;
}

function mockManagedRootsForRequest(request: ManagedScanRequest): ScanRootDto[] {
  const roots = request.roots.length ? request.roots : ["C:/Users/Zen"];
  const now = Math.floor(Date.now() / 1000);
  return roots.map((root, index) => ({
    id: `mock-scan-root-${index}`,
    normalizedPath: root.replaceAll("\\\\", "/"),
    displayName: root.split(/[\\/]/).filter(Boolean).at(-1) ?? root,
    sourceKind: "file_library",
    enabled: true,
    healthStatus: "healthy",
    currentGeneration: 1,
    activeRunId: null,
    activeGeneration: null,
    revision: 2,
    lastSuccessfulGeneration: 1,
    lastFullScanAt: now,
    needsReconciliation: false,
    lastErrorCode: null,
    lastErrorMessage: null,
    watcherRevision: 1,
    watcherAppliedRevision: 1,
    watcherLastEventAt: now,
    watcherLastAppliedAt: now,
    watcherLastErrorCode: null,
    watcherLastErrorMessage: null,
    watcherRuleRecoveryRequired: false,
    createdAt: now,
    updatedAt: now
  }));
}

function mockManagedScanRoots(): ScanRootDto[] {
  return mockManagedRootsForRequest(mockManagedScanState?.request ?? { roots: [], dedupe: false });
}

export function isTauriRuntimeUnavailable(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return message.includes("reading 'invoke'")
    || message.includes("reading \"invoke\"")
    || message.includes("reading 'listen'")
    || message.includes("reading \"listen\"")
    || message.includes("__TAURI_INTERNALS__")
    || message.includes("Tauri");
}

export { isBrowserMockEnabled } from "../utils/runtimeMode";

function isW201VirtualizationFixtureEnabled() {
  return typeof window !== "undefined"
    && new URLSearchParams(window.location.search).get("w2-01-browser-fixture") === "virtualized";
}

function isW205InteractionFixtureEnabled() {
  return typeof window !== "undefined"
    && new URLSearchParams(window.location.search).get("w2-05-browser-fixture") === "interaction";
}

function isW206GridFixtureEnabled() {
  return typeof window !== "undefined"
    && new URLSearchParams(window.location.search).get("w2-06-browser-fixture") === "grid";
}

function isW211IntegratedFixtureEnabled() {
  return typeof window !== "undefined"
    && new URLSearchParams(window.location.search).get("w2-11-browser-fixture") === "integrated";
}

function isW205StaleFixtureEnabled() {
  return typeof window !== "undefined"
    && new URLSearchParams(window.location.search).get("w2-05-browser-stale") === "true";
}

function queryMockFiles(args?: Record<string, unknown>): FileQueryResult {
  const limit = Number(args?.limit ?? 50);
  const offset = Number(args?.offset ?? 0);
  const query = String(args?.query ?? "").trim().toLowerCase();
  const filter = args?.filter as FileLibraryFilters | null | undefined;
  const filtered = applyLibraryFilter(
    query
      ? mockFiles.filter((item) => `${item.name} ${item.path} ${item.purpose}`.toLowerCase().includes(query))
      : mockFiles,
    filter?.libraryFilter
  );

  return {
    files: filtered.slice(offset, offset + limit),
    total: filtered.length,
    limit,
    offset
  };
}

function queryMockFileLibraryV2(args?: Record<string, unknown>): FileQueryResponseV2 {
  const request = args?.request as FileQueryRequestV2 | undefined;
  if (!request || request.version !== 2) throw new Error("library_query_version_unsupported");
  const fingerprint = mockLibraryFingerprint(request.query);
  const cursor = decodeMockLibraryCursor(request.cursor ?? null);
  if (cursor && (cursor.fingerprint !== fingerprint || cursor.revision !== mockLibraryRevision)) {
    return {
      version: 2,
      requestId: request.requestId,
      queryFingerprint: fingerprint,
      snapshotRevision: mockLibraryRevision,
      files: [],
      totalCount: null,
      countState: "deferred",
      countToken: null,
      nextCursor: null,
      hasMore: false,
      resultState: "snapshot_expired",
      scopeHealth: mockLibraryScopeHealth(request.query.scope)
    };
  }
  if (isW211IntegratedFixtureEnabled()) {
    return queryW211FileLibrary(request, fingerprint, cursor);
  }
  if (isW205InteractionFixtureEnabled()) {
    if (isW206GridFixtureEnabled()) {
      const stats = window as Window & { __zcW206LibraryPageCalls?: number };
      stats.__zcW206LibraryPageCalls = (stats.__zcW206LibraryPageCalls ?? 0) + 1;
    }
    const totalCount = 100_000;
    const pageSize = Math.max(1, Math.min(200, Number(request.pageSize) || 50));
    const start = cursor?.offset ?? 0;
    const pageLength = Math.max(0, Math.min(pageSize, totalCount - start));
    const files = Array.from({ length: pageLength }, (_, offset) => toW205FixtureFile(start + offset));
    const nextOffset = start + files.length;
    const hasMore = nextOffset < totalCount;
    return {
      version: 2,
      requestId: request.requestId,
      queryFingerprint: fingerprint,
      snapshotRevision: mockLibraryRevision,
      files: files.map(toMockFileLibrarySummary),
      totalCount,
      countState: "exact",
      countToken: null,
      nextCursor: hasMore ? encodeMockLibraryCursor({ fingerprint, revision: mockLibraryRevision, offset: nextOffset }) : null,
      hasMore,
      resultState: "complete",
      scopeHealth: mockLibraryScopeHealth(request.query.scope)
    };
  }
  const filtered = filterMockLibraryFiles(request.query);
  const sorted = [...filtered].sort((left, right) => compareMockLibraryFiles(left, right, request.query));
  const pageSize = Math.max(1, Math.min(200, Number(request.pageSize) || 50));
  const start = cursor?.offset ?? 0;
  const page = sorted.slice(start, start + pageSize);
  const hasMore = start + page.length < sorted.length;
  return {
    version: 2,
    requestId: request.requestId,
    queryFingerprint: fingerprint,
    snapshotRevision: mockLibraryRevision,
    files: page.map(toMockFileLibrarySummary),
    totalCount: sorted.length,
    countState: "exact",
    countToken: null,
    nextCursor: hasMore ? encodeMockLibraryCursor({
      fingerprint,
      revision: mockLibraryRevision,
      offset: start + page.length
    }) : null,
    hasMore,
    resultState: sorted.length ? "complete" : "empty",
    scopeHealth: mockLibraryScopeHealth(request.query.scope)
  };
}

function queryW211FileLibrary(
  request: FileQueryRequestV2,
  fingerprint: string,
  cursor: { fingerprint: string; revision: number; offset: number } | null
): FileQueryResponseV2 {
  const text = request.query.text?.trim().toLowerCase() ?? "";
  const totalCount = w211LibraryMatchCount(text);
  const pageSize = Math.max(1, Math.min(200, Number(request.pageSize) || 50));
  const start = cursor?.offset ?? 0;
  const pageLength = Math.max(0, Math.min(pageSize, totalCount - start));
  const files = Array.from({ length: pageLength }, (_, offset) => toW211FixtureFile(start + offset, text));
  const nextOffset = start + files.length;
  const hasMore = nextOffset < totalCount;
  const testWindow = typeof window === "undefined" ? null : window as Window & {
    __zcW211?: {
      libraryPageCalls: number;
      libraryPageLengths: number[];
      libraryQueries: string[];
      libraryFingerprints: string[];
      librarySnapshotRevisions: number[];
      selectionSummaries: Array<{ count: number; queryFingerprint: string | null; snapshotRevision: number }>;
    };
  };
  if (testWindow) {
    testWindow.__zcW211 ??= {
      libraryPageCalls: 0,
      libraryPageLengths: [],
      libraryQueries: [],
      libraryFingerprints: [],
      librarySnapshotRevisions: [],
      selectionSummaries: []
    };
    testWindow.__zcW211.libraryPageCalls += 1;
    testWindow.__zcW211.libraryPageLengths.push(files.length);
    testWindow.__zcW211.libraryQueries.push(text);
    testWindow.__zcW211.libraryFingerprints.push(fingerprint);
    testWindow.__zcW211.librarySnapshotRevisions.push(mockLibraryRevision);
  }
  return {
    version: 2,
    requestId: request.requestId,
    queryFingerprint: fingerprint,
    snapshotRevision: mockLibraryRevision,
    files: files.map(toMockFileLibrarySummary),
    totalCount,
    countState: "exact",
    countToken: null,
    nextCursor: hasMore ? encodeMockLibraryCursor({
      fingerprint,
      revision: mockLibraryRevision,
      offset: nextOffset
    }) : null,
    hasMore,
    resultState: totalCount > 0 ? "complete" : "empty",
    scopeHealth: mockLibraryScopeHealth(request.query.scope)
  };
}

function w211LibraryMatchCount(text: string) {
  if (text.length === 0) return W211_LIBRARY_TOTAL;
  if (text === "late") return 256;
  if (text === "slow-a" || text === "slow-b") return 1;
  if (text === "impossible-match") return 0;
  return 0;
}

function toW205FixtureFile(index: number): FileRecord {
  const sequence = String(index + 1).padStart(6, "0");
  return file({
    id: `w2-05-interaction-file-${sequence}`,
    name: `interaction-library-item-${sequence}.txt`,
    path: `C:/Users/Zen/Documents/interaction-library-item-${sequence}.txt`,
    directory: "C:/Users/Zen/Documents",
    extension: "txt",
    size: 4_096 + index,
    file_type: "Document",
    purpose: "Document",
    lifecycle: "Active",
    context: "W2-05 interaction convergence fixture",
    is_stale: isW205StaleFixtureEnabled() && index === 0
  });
}

function toW211FixtureFile(index: number, text: string): FileRecord {
  const sequence = String(index + 1).padStart(6, "0");
  const prefix = text === "late" ? "late-library-item"
    : text === "slow-a" ? "slow-a-library-item"
      : text === "slow-b" ? "slow-b-library-item"
        : "w2-11-library-item";
  return file({
    id: `w2-11-library-file-${sequence}`,
    name: `${prefix}-${sequence}.txt`,
    path: `C:/Users/Zen/Documents/${prefix}-${sequence}.txt`,
    directory: "C:/Users/Zen/Documents",
    extension: "txt",
    size: 4_096 + index,
    file_type: "Document",
    purpose: "Document",
    lifecycle: "Active",
    context: "W2-11 integrated performance fixture"
  });
}

function mockLibraryScopeHealth(scope: FileQueryRequestV2["query"]["scope"]): FileQueryResponseV2["scopeHealth"] {
  const roots = scope.kind === "roots"
    ? scope.scanRootIds.map((id) => mockLibraryRootHealth(id))
    : [mockLibraryRootHealth("mock-scan-root-0")];
  return {
    state: roots.every((root) => root.available && root.enabled) ? "healthy" : "partial",
    roots,
    invalidReferences: [],
    message: null
  };
}

function mockLibraryRootHealth(id: string) {
  return {
    id,
    displayName: id === "mock-scan-root-0" ? "Browser preview" : id,
    healthStatus: "ready",
    enabled: true,
    available: true,
    generation: 1,
    message: null
  };
}

function filterMockLibraryFiles(query: FileQueryRequestV2["query"]) {
  const filters = query.filters;
  const text = query.text?.trim().toLowerCase() ?? "";
  return mockFiles.filter((file) => {
    if (text && !`${file.name} ${file.path} ${file.purpose}`.toLowerCase().includes(text)) return false;
    if (filters.fileTypes?.length && !filters.fileTypes.includes(file.file_type)) return false;
    if (filters.purposes?.length && !filters.purposes.includes(file.purpose)) return false;
    if (filters.lifecycles?.length && !filters.lifecycles.includes(file.lifecycle)) return false;
    if (filters.risks?.length && !filters.risks.includes(file.risk_level)) return false;
    if (filters.sizeMin !== null && filters.sizeMin !== undefined && file.size < filters.sizeMin) return false;
    if (filters.sizeMax !== null && filters.sizeMax !== undefined && file.size > filters.sizeMax) return false;
    const modifiedAt = mockFileTimestamp(file.modified_at);
    const createdAt = mockFileTimestamp(file.created_at);
    if (filters.modifiedFrom !== null && filters.modifiedFrom !== undefined && modifiedAt < filters.modifiedFrom) return false;
    if (filters.modifiedTo !== null && filters.modifiedTo !== undefined && modifiedAt > filters.modifiedTo) return false;
    if (filters.createdFrom !== null && filters.createdFrom !== undefined && createdAt < filters.createdFrom) return false;
    if (filters.createdTo !== null && filters.createdTo !== undefined && createdAt > filters.createdTo) return false;
    if (filters.duplicate === "only" && !file.is_duplicate) return false;
    if (filters.duplicate === "exclude" && file.is_duplicate) return false;
    if (filters.review === "only" && !file.requires_confirmation) return false;
    if (filters.review === "exclude" && file.requires_confirmation) return false;
    const tags = mockFileTagIds.get(file.id) ?? new Set<string>();
    if (filters.tagsAllOf?.some((tagId) => !tags.has(tagId))) return false;
    if (filters.tagsAnyOf?.length && !filters.tagsAnyOf.some((tagId) => tags.has(tagId))) return false;
    if (filters.tagsNoneOf?.some((tagId) => tags.has(tagId))) return false;
    return !file.is_stale;
  });
}

function compareMockLibraryFiles(left: FileRecord, right: FileRecord, query: FileQueryRequestV2["query"]) {
  const kind = query.sort.kind;
  const leftValue = kind === "relevance"
    ? mockRelevance(left, query.text)
    : kind === "name"
      ? left.name.toLocaleLowerCase()
      : kind === "size"
        ? left.size
        : kind === "confidence"
          ? left.confidence
          : kind === "created"
            ? mockFileTimestamp(left.created_at)
            : mockFileTimestamp(left.modified_at);
  const rightValue = kind === "relevance"
    ? mockRelevance(right, query.text)
    : kind === "name"
      ? right.name.toLocaleLowerCase()
      : kind === "size"
        ? right.size
        : kind === "confidence"
          ? right.confidence
          : kind === "created"
            ? mockFileTimestamp(right.created_at)
            : mockFileTimestamp(right.modified_at);
  const comparison = leftValue < rightValue ? -1 : leftValue > rightValue ? 1 : 0;
  const directed = query.sort.direction === "desc" ? -comparison : comparison;
  return directed || left.id.localeCompare(right.id);
}

function mockRelevance(file: FileRecord, text: string | null) {
  const query = text?.trim().toLowerCase() ?? "";
  if (!query) return 0;
  return file.name.toLowerCase() === query ? 3 : file.name.toLowerCase().includes(query) ? 2 : 1;
}

function toMockFileLibrarySummary(file: FileRecord): FileLibrarySummary {
  return {
    id: file.id,
    name: file.name,
    extension: file.extension,
    displayDirectory: file.directory,
    size: file.size,
    modifiedAt: mockFileTimestamp(file.modified_at),
    createdAt: mockFileTimestamp(file.created_at),
    isDirectory: false,
    fileType: file.file_type,
    purpose: file.purpose,
    lifecycle: file.lifecycle,
    risk: file.risk_level,
    confidence: file.confidence,
    isDuplicate: file.is_duplicate,
    requiresReview: file.requires_confirmation,
    isStale: Boolean(file.is_stale),
    tags: mockFileTags(file.id),
    tagCount: (mockFileTagIds.get(file.id) ?? new Set()).size
  };
}

function getMockFileLibraryDetail(fileId: string): FileLibraryDetail {
  const interactionFixtureMatch = /^w2-05-interaction-file-(\d{6})$/u.exec(fileId);
  const file = interactionFixtureMatch && isW205InteractionFixtureEnabled()
    ? toW205FixtureFile(Number(interactionFixtureMatch[1]) - 1)
    : mockFiles.find((item) => item.id === fileId);
  if (!file) throw new Error("library_file_not_found");
  return {
    id: file.id,
    name: file.name,
    path: file.path,
    directory: file.directory,
    extension: file.extension,
    size: file.size,
    modifiedAt: mockFileTimestamp(file.modified_at),
    createdAt: mockFileTimestamp(file.created_at),
    isDirectory: false,
    fileType: file.file_type,
    purpose: file.purpose,
    lifecycle: file.lifecycle,
    context: file.context,
    risk: file.risk_level,
    confidence: file.confidence,
    classificationStatus: file.classification_status,
    classificationReason: file.classification_reason,
    matchedRules: [...file.matched_rules],
    suggestedAction: file.suggested_action,
    suggestedTargetPath: file.suggested_target_path,
    suggestedName: file.suggested_name,
    isDuplicate: file.is_duplicate,
    requiresReview: file.requires_confirmation,
    isStale: Boolean(file.is_stale),
    lastSeenAt: mockFileTimestamp(file.last_seen_at),
    scanRootId: "mock-scan-root-0",
    scanRootName: "Browser preview",
    scopeHealth: "ready",
    duplicateGroupId: file.is_duplicate ? "mock-duplicate-group" : null,
    duplicateGroupSize: file.is_duplicate ? 2 : 0,
    tags: mockFileTags(file.id),
    activeFindings: [],
    safeActions: file.is_stale ? [] : ["preview", "reveal"],
    revision: mockLibraryRevision
  };
}

function getMockFileLibrarySelectionSummary(selection?: LibrarySelectionV1): FileLibrarySelectionSummary {
  if (!selection) {
    return {
      count: 0,
      totalSize: 0,
      typeCounts: [],
      missingCount: 0,
      staleCount: 0,
      excludedCount: 0,
      commonDirectory: null,
      commonTags: [],
      commonTagIds: [],
      partialTagCommonalityCount: 0,
      snapshotRevision: mockLibraryRevision,
      queryFingerprint: null
    };
  }
  if (isW211IntegratedFixtureEnabled() && selection.kind === "all_matching") {
    const text = selection.query.text?.trim().toLowerCase() ?? "";
    const count = w211LibraryMatchCount(text);
    if (selection.queryFingerprint !== mockLibraryFingerprint(selection.query)
      || selection.snapshotRevision !== mockLibraryRevision) {
      throw new Error("library_snapshot_expired");
    }
    const excludedCount = selection.excludedFileIds.length;
    const effectiveCount = Math.max(0, count - excludedCount);
    const testWindow = typeof window === "undefined" ? null : window as Window & {
      __zcW211?: {
        selectionSummaries: Array<{ count: number; queryFingerprint: string | null; snapshotRevision: number }>;
      };
    };
    testWindow?.__zcW211?.selectionSummaries.push({
      count: effectiveCount,
      queryFingerprint: mockLibraryFingerprint(selection.query),
      snapshotRevision: mockLibraryRevision
    });
    return {
      count: effectiveCount,
      totalSize: effectiveCount * 4_096,
      typeCounts: effectiveCount > 0 ? [{ fileType: "Document", count: effectiveCount }] : [],
      missingCount: 0,
      staleCount: 0,
      excludedCount,
      commonDirectory: effectiveCount > 0 ? "C:/Users/Zen/Documents" : null,
      commonTags: [],
      commonTagIds: [],
      partialTagCommonalityCount: 0,
      snapshotRevision: mockLibraryRevision,
      queryFingerprint: mockLibraryFingerprint(selection.query)
    };
  }
  let files: FileRecord[];
  let excludedCount = 0;
  let queryFingerprint: string | null = null;
  if (selection.kind === "explicit") {
    const requested = new Set(selection.fileIds);
    files = mockFiles.filter((file) => requested.has(file.id) && !file.is_stale);
    excludedCount = selection.fileIds.filter((id) => !files.some((file) => file.id === id)).length;
  } else {
    queryFingerprint = mockLibraryFingerprint(selection.query);
    if (queryFingerprint !== selection.queryFingerprint || selection.snapshotRevision !== mockLibraryRevision) {
      throw new Error("library_snapshot_expired");
    }
    const excluded = new Set(selection.excludedFileIds);
    files = filterMockLibraryFiles(selection.query).filter((file) => !excluded.has(file.id));
    excludedCount = selection.excludedFileIds.filter((id) => !files.some((file) => file.id === id)).length;
  }
  const typeCounts = [...files.reduce((counts, file) => counts.set(file.file_type, (counts.get(file.file_type) ?? 0) + 1), new Map<string, number>())]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([fileType, count]) => ({ fileType, count }));
  return {
    count: files.length,
    totalSize: files.reduce((sum, file) => sum + file.size, 0),
    typeCounts,
    missingCount: 0,
    staleCount: selection.kind === "explicit" ? selection.fileIds.filter((id) => mockFiles.some((file) => file.id === id && file.is_stale)).length : 0,
    excludedCount,
    commonDirectory: files.length && files.every((file) => file.directory === files[0].directory) ? files[0].directory : null,
    commonTags: [],
    commonTagIds: [],
    partialTagCommonalityCount: 0,
    snapshotRevision: mockLibraryRevision,
    queryFingerprint
  };
}

function createMockUserTag(request?: CreateUserTagRequest): UserTag {
  const displayName = String(request?.displayName ?? "").trim();
  if (!displayName) throw new Error("library_tag_name_invalid");
  if (mockUserTags.some((tag) => tag.displayName.toLowerCase() === displayName.toLowerCase())) throw new Error("library_tag_conflict");
  const timestamp = Math.floor(Date.now() / 1000);
  const tag: UserTag = {
    id: `browser-user-tag-${Date.now()}`,
    displayName,
    colorToken: request?.colorToken ?? "neutral",
    usageCount: 0,
    createdAt: timestamp,
    updatedAt: timestamp,
    revision: 1
  };
  mockUserTags = [...mockUserTags, tag];
  mockLibraryRevision += 1;
  return tag;
}

function updateMockUserTag(request?: UpdateUserTagRequest): UserTag {
  const index = mockUserTags.findIndex((tag) => tag.id === request?.id);
  if (index < 0) throw new Error("library_tag_not_found");
  const current = mockUserTags[index];
  if (request?.expectedRevision !== current.revision) throw new Error("library_tag_stale_or_missing");
  const updated = { ...current, displayName: String(request?.displayName ?? "").trim(), colorToken: request?.colorToken ?? current.colorToken, updatedAt: Math.floor(Date.now() / 1000), revision: current.revision + 1 };
  mockUserTags = mockUserTags.map((tag, itemIndex) => itemIndex === index ? updated : tag);
  mockLibraryRevision += 1;
  return updated;
}

function deleteMockUserTag(request?: DeleteUserTagRequest): boolean {
  const tag = mockUserTags.find((item) => item.id === request?.id);
  if (!tag) throw new Error("library_tag_not_found");
  if (!request?.confirm) throw new Error("library_tag_delete_confirmation_required");
  if (tag.usageCount !== request.expectedUsageCount) throw new Error("library_tag_stale_usage");
  if (tag.revision !== request.expectedRevision) throw new Error("library_tag_stale_usage");
  mockUserTags = mockUserTags.filter((item) => item.id !== tag.id);
  for (const ids of mockFileTagIds.values()) ids.delete(tag.id);
  mockLibraryRevision += 1;
  refreshMockTagUsage();
  return true;
}

function mutateMockFileUserTags(request?: MutateFileUserTagsRequest): MutateFileUserTagsResult {
  if (!request?.tagIds?.length) throw new Error("library_tag_mutation_invalid_tags");
  const selection = request.selection;
  const summary = getMockFileLibrarySelectionSummary(selection);
  const targetFiles = selection.kind === "explicit"
    ? mockFiles.filter((file) => selection.fileIds.includes(file.id) && !file.is_stale)
    : filterMockLibraryFiles(selection.query).filter((file) => !selection.excludedFileIds.includes(file.id));
  let appliedCount = 0;
  let alreadyPresentCount = 0;
  for (const file of targetFiles) {
    const ids = mockFileTagIds.get(file.id) ?? new Set<string>();
    mockFileTagIds.set(file.id, ids);
    for (const tagId of request.tagIds) {
      if (!mockUserTags.some((tag) => tag.id === tagId)) throw new Error("library_tag_not_found");
      if (request.operation === "add") {
        if (ids.has(tagId)) alreadyPresentCount += 1;
        else { ids.add(tagId); appliedCount += 1; }
      } else if (ids.delete(tagId)) {
        appliedCount += 1;
      }
    }
  }
  if (appliedCount) mockLibraryRevision += 1;
  refreshMockTagUsage();
  return {
    appliedCount,
    alreadyPresentCount,
    missingCount: summary.missingCount,
    excludedCount: summary.excludedCount,
    revision: mockLibraryRevision
  };
}

function refreshMockTagUsage() {
  mockUserTags = mockUserTags.map((tag) => ({
    ...tag,
    usageCount: [...mockFileTagIds.values()].filter((ids) => ids.has(tag.id)).length
  }));
}

function createMockLibrarySavedView(request?: CreateLibrarySavedViewRequest): LibrarySavedView {
  const displayName = String(request?.displayName ?? "").trim();
  if (!displayName) throw new Error("library_saved_view_name_invalid");
  if (!request?.query) throw new Error("library_saved_view_query_invalid");
  if (mockLibrarySavedViews.some((view) => view.displayName.toLowerCase() === displayName.toLowerCase())) throw new Error("library_saved_view_conflict");
  const timestamp = Math.floor(Date.now() / 1000);
  const view: LibrarySavedView = {
    id: `browser-library-view-${Date.now()}`,
    displayName,
    query: request.query,
    queryFingerprint: mockLibraryFingerprint(request?.query),
    position: Math.max(0, Number(request?.position ?? 0)),
    createdAt: timestamp,
    updatedAt: timestamp,
    invalidReferences: invalidMockSavedViewReferences(request?.query),
    revision: 1
  };
  mockLibrarySavedViews = [...mockLibrarySavedViews, view];
  return view;
}

function updateMockLibrarySavedView(request?: UpdateLibrarySavedViewRequest): LibrarySavedView {
  const index = mockLibrarySavedViews.findIndex((view) => view.id === request?.id);
  if (index < 0) throw new Error("library_saved_view_stale_or_missing");
  const current = mockLibrarySavedViews[index];
  if (current.revision !== request?.expectedRevision) throw new Error("library_saved_view_stale_or_missing");
  const updated: LibrarySavedView = {
    ...current,
    displayName: String(request?.displayName ?? "").trim(),
    query: request?.query,
    queryFingerprint: mockLibraryFingerprint(request?.query),
    position: Math.max(0, Number(request?.position ?? 0)),
    updatedAt: Math.floor(Date.now() / 1000),
    invalidReferences: invalidMockSavedViewReferences(request?.query),
    revision: current.revision + 1
  };
  mockLibrarySavedViews = mockLibrarySavedViews.map((view, itemIndex) => itemIndex === index ? updated : view);
  return updated;
}

function deleteMockLibrarySavedView(request?: DeleteLibrarySavedViewRequest): boolean {
  const view = mockLibrarySavedViews.find((item) => item.id === request?.id);
  if (!view || view.revision !== request?.expectedRevision) throw new Error("library_saved_view_stale_or_missing");
  mockLibrarySavedViews = mockLibrarySavedViews.filter((item) => item.id !== view.id);
  return true;
}

interface MockOrganizationDecisionRequest {
  planId?: string;
  expectedPlanRevision?: number;
  safeBatch?: boolean;
  mutations?: Array<{
    itemId: string;
    expectedItemRevision: number;
    decision: OrganizationPlanItem["decision"];
    editedFilename?: string | null;
  }>;
}

function createMockOrganizationPlan(request?: { title?: string; source?: LibrarySelectionV1; expectedCount?: number }, automationRunId?: string): OrganizationPlan {
  if (!request?.source) throw new Error("organization_plan_request_invalid");
  const source = request.source;
  const summary = getMockFileLibrarySelectionSummary(source);
  if (request.expectedCount !== undefined && request.expectedCount !== summary.count) throw new Error("organization_plan_expected_count_mismatch");
  if (summary.count > 10_000) throw new Error("organization_plan_too_large");
  const timestamp = Math.floor(Date.now() / 1000);
  const presentationOnly = new URLSearchParams(globalThis.location?.search ?? "").has("pm01-organize");
  const id = `${presentationOnly ? "browser-presentation-organization-plan" : "browser-organization-plan"}-${automationRunId ?? Date.now()}`;
  const plan: OrganizationPlan = {
    id,
    title: request.title?.trim() || "Organization plan",
    status: "ready",
    sourceKind: source.kind,
    sourceQueryFingerprint: source.kind === "all_matching" ? source.queryFingerprint : null,
    sourceSnapshotRevision: source.kind === "all_matching" ? source.snapshotRevision : mockLibraryRevision,
    requestedCount: summary.count,
    materializedCount: summary.count,
    plannerVersion: 1,
    revision: 1,
    activeExecutionId: null,
    activeOperationBatchId: null,
    lastErrorCode: null,
    lastErrorDetail: null,
    createdAt: timestamp,
    updatedAt: timestamp,
    readyAt: timestamp,
    completedAt: null,
    summary: emptyMockOrganizationSummary(),
    effectiveSummary: null
  };
  const sourceFiles = source.kind === "explicit"
    ? mockFiles.filter((file) => source.fileIds.includes(file.id) && !file.is_stale)
    : filterMockLibraryFiles(source.query).filter((file) => !source.excludedFileIds.includes(file.id));
  mockOrganizationItems.set(id, sourceFiles.sort((left, right) => left.id.localeCompare(right.id)).map((file, ordinal) => {
    // This is a deterministic display fixture selected by the browser URL. It
    // intentionally never reads files.suggested_* or claims AI authority.
    const state = presentationOnly ? mockOrganizationPresentationState() : "needs-analysis";
    const actionable = state === "ready" || state === "review";
    const blocked = state === "blocked";
    const validity: OrganizationPlanItem["validity"] = blocked ? "blocked" : state === "needs-analysis" ? "needs_analysis" : state === "review" ? "needs_review" : "ready";
    const proposalKind: OrganizationPlanItem["proposalKind"] = blocked ? "blocked" : actionable ? "move" : "keep";
    const confidence = state === "ready" ? 0.94 : state === "review" ? 0.68 : 0;
    const requiresConfirmation = state === "review";
    const proposedTargetDirectory = actionable ? `${file.directory.replace(/[\\/]$/, "")}/Presentation fixture` : file.directory;
    const proposedTargetPath = actionable ? `${proposedTargetDirectory}/${file.name}` : file.path;
    const previewId = actionable ? `browser-presentation-only-preview-${file.id}` : null;
    const blockingCode = blocked ? "browser_presentation_fixture_blocked" : null;
    const semanticExplanation = actionable ? {
      assessmentFingerprint: `browser-presentation-only:${file.id}`,
      reason: "Browser presentation fixture only; no current backend SemanticAssessmentV1 is present."
    } : null;
    return {
      id: `${id}-item-${ordinal}`,
      planId: id,
      ordinal,
      fileIdSnapshot: file.id,
      sourcePathSnapshot: file.path,
      sourceNameSnapshot: file.name,
      sourceSizeSnapshot: file.size,
      sourceMtimeSnapshot: mockFileTimestamp(file.modified_at),
      sourceIsDirSnapshot: false,
      proposalFingerprint: mockLibraryFingerprint(defaultFileLibraryQueryForMock(file.id)),
      semanticExplanation,
      proposalKind,
      proposedTargetDirectory,
      proposedName: file.name,
      proposedTargetPath,
      decision: "undecided",
      editedName: null,
      validity,
      reviewState: validity,
      effectiveReadiness: blocked || state === "needs-analysis" ? "blocked" : state === "review" ? "requires-decision" : "ready",
      confidence,
      riskLevel: "Normal",
      requiresConfirmation,
      blockingCode,
      blockingDetail: blocked ? "Browser presentation fixture state; backend authority is not available." : null,
      authoritativePreviewId: previewId,
      reviewReasons: mockOrganizationReviewReasons({
        validity,
        confidence,
        riskLevel: "Normal",
        requiresConfirmation,
        authoritativePreviewId: previewId,
        blockingCode,
        isDuplicate: false
      }),
      availableActions: mockOrganizationAvailableActions({
        validity,
        decision: "undecided",
        proposalKind,
        authoritativePreviewId: previewId
      }),
      operationLogId: null,
      executionId: null,
      revision: 1,
      createdAt: timestamp,
      updatedAt: timestamp
    } satisfies OrganizationPlanItem;
  }));
  plan.summary = mockOrganizationSummary(mockOrganizationItems.get(id) ?? []);
  plan.effectiveSummary = mockOrganizationEffectiveSummary(mockOrganizationItems.get(id) ?? []);
  mockOrganizationPlans = [plan, ...mockOrganizationPlans];
  return plan;
}

function mockOrganizationPresentationState(): "needs-analysis" | "ready" | "review" | "blocked" {
  const value = new URLSearchParams(globalThis.location?.search ?? "").get("pm01-organize");
  return value === "ready" || value === "review" || value === "blocked" ? value : "needs-analysis";
}

function queryMockOrganizationItems(request?: { planId?: string; cursor?: string | null; pageSize?: number }) {
  const plan = mockOrganizationPlans.find((item) => item.id === request?.planId);
  if (!plan) throw new Error("organization_plan_not_found");
  const all = mockOrganizationItems.get(plan.id) ?? [];
  const offset = Number(request?.cursor ?? 0) || 0;
  const pageSize = Math.max(1, Math.min(200, Number(request?.pageSize ?? 100)));
  const items = all.slice(offset, offset + pageSize);
  const next = offset + items.length;
  return {
    planId: plan.id,
    planRevision: plan.revision,
    items,
    nextCursor: next < all.length ? String(next) : null,
    hasMore: next < all.length
  };
}

function mockOrganizationGroupReadiness(item: OrganizationPlanItem): OrganizationPlanGroupSummary["readiness"] {
  return mockOrganizationEffectiveReadiness(item);
}

function mockOrganizationEffectiveReadiness(item: OrganizationPlanItem): OrganizationPlanGroupSummary["readiness"] {
  if (!(["ready", "needs_review"] as OrganizationPlanItem["validity"][]).includes(item.validity)) return "blocked";
  if (["source_identity_changed", "source_missing", "managed_scope_unavailable", "managed_scope_membership_changed", "live_proposal_changed", "proposal_changed", "cleanup_review_required"].includes(item.blockingCode ?? "")) return "blocked";
  if (item.proposalKind !== "keep" && !item.authoritativePreviewId) return "blocked";
  if (item.validity === "needs_review") {
    return item.decision === "undecided" ? "requires-decision" : "reviewed";
  }
  return "ready";
}

function mockOrganizationReviewReasons(input: {
  validity: OrganizationPlanItem["validity"];
  confidence: number;
  riskLevel: string;
  requiresConfirmation: boolean;
  authoritativePreviewId: string | null;
  blockingCode: string | null;
  isDuplicate: boolean;
}): string[] {
  const reasons: string[] = [];
  if (input.validity !== "ready" && input.confidence < 0.8) reasons.push("low_confidence");
  if (input.riskLevel === "Sensitive") reasons.push("sensitive_file");
  if (input.riskLevel !== "Normal" && input.riskLevel) reasons.push("non_normal_risk");
  if (input.requiresConfirmation) reasons.push("requires_confirmation");
  if (input.isDuplicate) reasons.push("possible_duplicate");
  if (input.blockingCode === "cleanup_review_required") reasons.push("unsupported_operation");
  if (!input.authoritativePreviewId && input.validity !== "needs_analysis") reasons.push("missing_preview");
  return reasons.length ? reasons : input.validity === "needs_review" ? ["requires_confirmation"] : [];
}

function mockOrganizationAvailableActions(input: {
  validity: OrganizationPlanItem["validity"];
  decision: OrganizationPlanItem["decision"];
  proposalKind: OrganizationPlanItem["proposalKind"];
  authoritativePreviewId: string | null;
  previewExecutable?: boolean;
  previewEditable?: boolean;
  blockingCode?: string | null;
}): string[] {
  const actions: string[] = [];
  const supported = ["move", "rename", "move_rename"].includes(input.proposalKind);
  const active = !["stale", "executing", "executed", "failed", "skipped", "needs_analysis"].includes(input.validity);
  const executable = input.previewExecutable ?? !["target_collision", "extension_change_blocked", "unsafe_filename", "sensitive_file", "unsupported_operation"].includes(input.blockingCode ?? "");
  const editable = input.previewEditable ?? (input.blockingCode !== "extension_change_blocked" && input.blockingCode !== "unsafe_filename");
  const hardBlocked = ["source_identity_changed", "source_missing", "managed_scope_unavailable", "managed_scope_membership_changed", "live_proposal_changed", "proposal_changed", "cleanup_review_required"].includes(input.blockingCode ?? "");
  const reviewable = input.validity === "ready" || input.validity === "needs_review";
  const collisionEditable = input.validity === "blocked" && input.blockingCode === "target_collision";
  if (active && supported && input.authoritativePreviewId && executable && reviewable && !hardBlocked && input.decision === "undecided") actions.push("accept_suggestion");
  if (active && supported && input.authoritativePreviewId && editable && !hardBlocked && (reviewable || collisionEditable)) actions.push("edit_name", "view_preview");
  if (active) actions.push("keep");
  if (active && input.decision !== "undecided") actions.push("clear_decision");
  if (active && input.validity === "needs_review" && input.decision === "undecided") actions.push("defer");
  return actions;
}

function mockOrganizationGroupId(planId: string, item: OrganizationPlanItem) {
  return [
    "browser-organization-group",
    planId,
    item.proposedTargetDirectory,
    item.proposalKind,
    mockOrganizationGroupReadiness(item),
    item.riskLevel
  ].join("|");
}

function mockOrganizationGroupProjectionFingerprint(plan: OrganizationPlan, groupId: string, members: OrganizationPlanItem[]) {
  const fingerprintMembers = [...members]
    .sort((left, right) => left.id.localeCompare(right.id))
    .map((item) => ({
      itemId: item.id,
      itemRevision: item.revision,
      effectiveReadiness: item.effectiveReadiness,
      decision: item.decision,
      authoritativePreviewId: item.authoritativePreviewId,
      currentSourcePath: item.sourcePathSnapshot,
      currentSize: item.sourceSizeSnapshot,
      currentMtime: item.sourceMtimeSnapshot,
      currentIsDir: item.sourceIsDirSnapshot,
      availableActions: [...item.availableActions].sort(),
      proposalFingerprint: item.proposalFingerprint,
      blockingCode: item.blockingCode,
      managedScopeMembership: true
    }));
  const query = defaultFileLibraryQueryForMock(`${plan.id}:${groupId}`);
  return `browser-organization-group-projection-v1-${mockLibraryFingerprint({
    ...query,
    text: JSON.stringify({
      version: "browser-organization-group-projection-v1",
      planId: plan.id,
      planRevision: plan.revision,
      groupId,
      members: fingerprintMembers
    })
  })}`;
}

function mockOrganizationGroupSummaries(plan: OrganizationPlan): OrganizationPlanGroupSummary[] {
  const items = mockOrganizationItems.get(plan.id) ?? [];
  const grouped = new Map<string, OrganizationPlanItem[]>();
  for (const item of items) {
    const id = mockOrganizationGroupId(plan.id, item);
    const members = grouped.get(id) ?? [];
    members.push(item);
    grouped.set(id, members);
  }
  return [...grouped.entries()].map(([groupId, members]) => {
    const first = members[0];
    const confidences = members.map((item) => item.confidence);
    const allHigh = confidences.every((value) => value >= 0.8);
    const allMedium = confidences.every((value) => value >= 0.5 && value < 0.8);
    const allLow = confidences.every((value) => value < 0.5);
    return {
      groupId,
      planId: plan.id,
      label: `${first.proposedTargetDirectory} · ${first.proposalKind}`,
      targetDirectory: first.proposedTargetDirectory || null,
      proposalKind: first.proposalKind,
      readiness: mockOrganizationGroupReadiness(first),
      riskLevel: first.riskLevel,
      itemCount: members.length,
      totalBytes: members.reduce((sum, item) => sum + item.sourceSizeSnapshot, 0),
      acceptedCount: members.filter((item) => ["accepted", "edited"].includes(item.decision)).length,
      excludedCount: members.filter((item) => item.decision === "kept").length,
      staleCount: members.filter((item) => item.validity === "stale").length,
      conflictCount: members.filter((item) => item.blockingCode?.includes("collision") || item.blockingCode?.includes("conflict")).length,
      confidenceBand: allHigh ? "high" : allMedium ? "medium" : allLow ? "low" : "mixed",
      reviewReasonCounts: [...members.reduce((counts, item) => {
        for (const reason of item.reviewReasons) counts.set(reason, (counts.get(reason) ?? 0) + 1);
        return counts;
      }, new Map<string, number>())].map(([reason, count]) => ({ reason, count })).sort((left, right) => left.reason.localeCompare(right.reason)),
      availableActions: [...new Set(members.flatMap((item) => item.availableActions))],
      groupActions: {
        canAcceptAll: members.length > 0
          && members.every((item) => item.availableActions.includes("accept_suggestion")),
        canKeepAll: members.length > 0 && members.every((item) => item.availableActions.includes("keep")),
        canClearAll: members.length > 0 && members.every((item) => item.availableActions.includes("clear_decision"))
      },
      projectionFingerprint: mockOrganizationGroupProjectionFingerprint(plan, groupId, members),
      sampleItems: members.slice(0, 3).map((item) => ({
        itemId: item.id,
        sourceName: item.sourceNameSnapshot,
        sourcePath: item.sourcePathSnapshot,
        proposedName: item.proposedName,
        decision: item.decision,
        validity: item.validity
      })),
      revision: plan.revision
    } satisfies OrganizationPlanGroupSummary;
  }).sort((left, right) => left.label.localeCompare(right.label) || left.groupId.localeCompare(right.groupId));
}

function mockOrganizationPlanGroupProjectionFingerprint(
  plan: OrganizationPlan,
  groups: OrganizationPlanGroupSummary[]
): string {
  const query = defaultFileLibraryQueryForMock(`${plan.id}:organization-groups`);
  return `browser-organization-groups-projection-v1-${mockLibraryFingerprint({
    ...query,
    text: JSON.stringify({
      version: "browser-organization-groups-projection-v1",
      planId: plan.id,
      planRevision: plan.revision,
      groups: groups.map((group) => ({
        groupId: group.groupId,
        projectionFingerprint: group.projectionFingerprint
      }))
    })
  })}`;
}

function queryMockOrganizationGroups(request?: { planId?: string; cursor?: string | null; pageSize?: number }): OrganizationPlanGroupPage {
  const plan = mockOrganizationPlans.find((item) => item.id === request?.planId);
  if (!plan) throw new Error("organization_plan_not_found");
  const all = mockOrganizationGroupSummaries(plan);
  const projectionFingerprint = mockOrganizationPlanGroupProjectionFingerprint(plan, all);
  const offset = Number(request?.cursor ?? 0) || 0;
  const pageSize = Math.max(1, Math.min(200, Number(request?.pageSize ?? 100)));
  const groups = all.slice(offset, offset + pageSize);
  const next = offset + groups.length;
  return {
    planId: plan.id,
    planRevision: plan.revision,
    groups,
    effectiveSummary: mockOrganizationEffectiveSummary(mockOrganizationItems.get(plan.id) ?? []),
    projectionFingerprint,
    nextCursor: next < all.length ? String(next) : null,
    hasMore: next < all.length
  };
}

function queryMockOrganizationGroupItems(request?: { planId?: string; groupId?: string; cursor?: string | null; expectedProjectionFingerprint?: string; pageSize?: number }): OrganizationPlanGroupItemPage {
  const plan = mockOrganizationPlans.find((item) => item.id === request?.planId);
  if (!plan) throw new Error("organization_plan_not_found");
  const currentGroup = mockOrganizationGroupSummaries(plan).find((group) => group.groupId === request?.groupId);
  if (!currentGroup) throw new Error("organization_group_not_found");
  if (request?.expectedProjectionFingerprint && currentGroup.projectionFingerprint !== request.expectedProjectionFingerprint) {
    throw new Error("organization_group_projection_changed");
  }
  const all = (mockOrganizationItems.get(plan.id) ?? []).filter((item) => mockOrganizationGroupId(plan.id, item) === request?.groupId);
  if (!all.length) throw new Error("organization_group_not_found");
  const offset = Number(request?.cursor ?? 0) || 0;
  const pageSize = Math.max(1, Math.min(200, Number(request?.pageSize ?? 100)));
  const items = all.slice(offset, offset + pageSize);
  const next = offset + items.length;
  return {
    planId: plan.id,
    groupId: String(request?.groupId ?? ""),
    planRevision: plan.revision,
    projectionFingerprint: currentGroup.projectionFingerprint,
    items,
    nextCursor: next < all.length ? String(next) : null,
    hasMore: next < all.length
  };
}

function updateMockOrganizationGroupDecision(request?: {
  planId?: string;
  groupId?: string;
  expectedPlanRevision?: number;
  expectedProjectionFingerprint?: string;
  expectedItemCount?: number;
  decision?: OrganizationPlanItem["decision"];
}) {
  const plan = mockOrganizationPlans.find((item) => item.id === request?.planId);
  if (!plan || plan.revision !== request?.expectedPlanRevision) throw new Error("organization_plan_revision_conflict");
  const currentGroup = mockOrganizationGroupSummaries(plan).find((group) => group.groupId === request?.groupId);
  if (!currentGroup
    || currentGroup.itemCount !== request?.expectedItemCount
    || currentGroup.projectionFingerprint !== request?.expectedProjectionFingerprint) {
    throw new Error("organization_group_changed");
  }
  const members = (mockOrganizationItems.get(plan.id) ?? []).filter((item) => mockOrganizationGroupId(plan.id, item) === request?.groupId);
  const decision = request?.decision;
  if (members.some((item) => ["executing", "executed"].includes(item.validity))) throw new Error("organization_group_changed");
  const actionAvailable = decision === "accepted"
    ? currentGroup.groupActions.canAcceptAll
    : decision === "kept"
      ? currentGroup.groupActions.canKeepAll
      : currentGroup.groupActions.canClearAll;
  if (!actionAvailable) throw new Error("organization_group_action_not_available");
  const updated = updateMockOrganizationDecisions({
    planId: plan.id,
    expectedPlanRevision: plan.revision,
    safeBatch: false,
    mutations: members.map((item) => ({ itemId: item.id, expectedItemRevision: item.revision, decision: decision ?? "undecided" }))
  });
  return {
    plan: updated,
    group: null
  } satisfies { plan: OrganizationPlan; group: OrganizationPlanGroupSummary | null };
}

function updateMockOrganizationDecisions(request?: MockOrganizationDecisionRequest): OrganizationPlan {
  const plan = mockOrganizationPlans.find((item) => item.id === request?.planId);
  if (!plan || plan.revision !== request?.expectedPlanRevision) throw new Error("organization_plan_revision_conflict");
  const items = mockOrganizationItems.get(plan.id) ?? [];
  const stagedItems = items.map((item) => ({ ...item, availableActions: [...item.availableActions], reviewReasons: [...item.reviewReasons] }));
  for (const mutation of request?.mutations ?? []) {
    const item = stagedItems.find((candidate) => candidate.id === mutation.itemId);
    if (!item || item.revision !== mutation.expectedItemRevision) throw new Error("organization_item_revision_conflict");
    const availableActions = mockOrganizationAvailableActions({
      validity: item.validity,
      decision: item.decision,
      proposalKind: item.proposalKind,
      authoritativePreviewId: item.authoritativePreviewId,
      blockingCode: item.blockingCode
    });
    const requiredAction = mutation.decision === "accepted"
      ? "accept_suggestion"
      : mutation.decision === "edited"
        ? "edit_name"
        : mutation.decision === "kept"
          ? "keep"
          : item.decision === "undecided" ? null : "clear_decision";
    if (requiredAction && !availableActions.includes(requiredAction)) throw new Error(requiredAction === "accept_suggestion" ? "organization_item_accept_not_available" : requiredAction === "edit_name" ? "organization_item_edit_not_available" : "organization_item_action_not_available");
    if (request?.safeBatch && (
      mutation.decision !== "accepted"
      || item.validity !== "ready"
      || item.riskLevel !== "Normal"
      || item.confidence < 0.8
      || item.requiresConfirmation
      || item.blockingCode !== null
      || item.authoritativePreviewId === null
    )) {
      throw new Error("organization_safe_batch_item_blocked");
    }
    item.decision = mutation.decision;
    item.editedName = mutation.decision === "edited" ? mutation.editedFilename ?? null : null;
    item.availableActions = mockOrganizationAvailableActions({
      validity: item.validity,
      decision: item.decision,
      proposalKind: item.proposalKind,
      authoritativePreviewId: item.authoritativePreviewId,
      blockingCode: item.blockingCode
    });
    item.reviewState = item.validity === "needs_review" && ["accepted", "edited", "kept"].includes(item.decision)
      ? "reviewed"
      : item.validity;
    item.effectiveReadiness = mockOrganizationEffectiveReadiness(item);
    item.revision += 1;
  }
  mockOrganizationItems.set(plan.id, stagedItems);
  const updated = {
    ...plan,
    revision: plan.revision + 1,
    updatedAt: Math.floor(Date.now() / 1000),
    summary: mockOrganizationSummary(stagedItems),
    effectiveSummary: null
  };
  mockOrganizationPlans = mockOrganizationPlans.map((item) => item.id === plan.id ? updated : item);
  return updated;
}

function refreshMockOrganizationPlan(request?: { planId?: string; expectedPlanRevision?: number }): OrganizationPlan {
  const plan = mockOrganizationPlans.find((item) => item.id === request?.planId);
  if (!plan || plan.revision !== request?.expectedPlanRevision) throw new Error("organization_plan_revision_conflict");
  const updated = { ...plan, status: "ready" as const, revision: plan.revision + 1, updatedAt: Math.floor(Date.now() / 1000) };
  mockOrganizationPlans = mockOrganizationPlans.map((item) => item.id === plan.id ? updated : item);
  return updated;
}

function cancelMockOrganizationPlan(request?: { planId?: string; expectedPlanRevision?: number }): OrganizationPlan {
  const plan = mockOrganizationPlans.find((item) => item.id === request?.planId);
  if (!plan || plan.revision !== request?.expectedPlanRevision) throw new Error("organization_plan_revision_conflict");
  const updated = { ...plan, status: "cancelled" as const, revision: plan.revision + 1, updatedAt: Math.floor(Date.now() / 1000) };
  mockOrganizationPlans = mockOrganizationPlans.map((item) => item.id === plan.id ? updated : item);
  return updated;
}

function deleteMockOrganizationPlan(request?: { planId?: string; expectedPlanRevision?: number; confirmed?: boolean }): boolean {
  const plan = mockOrganizationPlans.find((item) => item.id === request?.planId);
  if (!request?.confirmed || !plan || plan.revision !== request.expectedPlanRevision || !["completed", "cancelled", "failed"].includes(plan.status)) {
    throw new Error("organization_plan_delete_blocked");
  }
  mockOrganizationPlans = mockOrganizationPlans.filter((item) => item.id !== plan.id);
  mockOrganizationItems.delete(plan.id);
  return true;
}

function getMockOrganizationDryRun(request?: { planId?: string; expectedPlanRevision?: number; itemIds?: string[]; allAccepted?: boolean }): OrganizationPlanDryRun {
  const plan = mockOrganizationPlans.find((item) => item.id === request?.planId);
  if (!plan || plan.revision !== request?.expectedPlanRevision) throw new Error("organization_plan_revision_conflict");
  const requested = new Set(request?.itemIds ?? []);
  const selected = (mockOrganizationItems.get(plan.id) ?? []).filter((item) =>
    ["accepted", "edited"].includes(item.decision) && (request?.allAccepted || requested.has(item.id))
  );
  const items = selected.map((item) => ({
    itemId: item.id,
    operationKind: item.proposalKind,
    from: item.sourcePathSnapshot,
    to: item.editedName ? `${item.proposedTargetDirectory}/${item.editedName}` : item.proposedTargetPath,
    editedFilename: item.editedName,
    parentDirectoryToCreate: null,
    collision: false,
    crossVolume: false,
    riskLevel: item.riskLevel,
    requiresConfirmation: item.requiresConfirmation,
    sourceHealth: "healthy",
    authoritativePreviewId: item.authoritativePreviewId,
    executable: ["ready", "reviewed"].includes(item.reviewState) && item.authoritativePreviewId !== null,
    blockingCode: item.blockingCode
  }));
  const executable = items.filter((item) => item.executable);
  return {
    planId: plan.id,
    planRevision: plan.revision,
    selectedCount: items.length,
    executableCount: executable.length,
    blockedCount: items.length - executable.length,
    staleCount: 0,
    totalBytes: selected.filter((_, index) => items[index]?.executable).reduce((sum, item) => sum + item.sourceSizeSnapshot, 0),
    operationKinds: [...new Set(executable.map((item) => item.operationKind))].sort(),
    items,
    executionBatchLimit: 1000,
    dryRunFingerprint: mockLibraryFingerprint(defaultFileLibraryQueryForMock(`${plan.id}:${plan.revision}:${items.map((item) => item.itemId).join(",")}`))
  };
}

function emptyMockOrganizationSummary(): OrganizationPlan["summary"] {
  return {
    undecided: 0,
    accepted: 0,
    kept: 0,
    edited: 0,
    needsAnalysis: 0,
    needsReview: 0,
    pendingReview: 0,
    reviewed: 0,
    ready: 0,
    blocked: 0,
    stale: 0,
    executing: 0,
    executed: 0,
    failed: 0,
    skipped: 0,
    remainingExecutable: 0
  };
}

function emptyMockOrganizationEffectiveSummary(): OrganizationPlanEffectiveSummary {
  return { ready: 0, reviewed: 0, pendingReview: 0, blocked: 0 };
}

function mockOrganizationEffectiveSummary(items: OrganizationPlanItem[]): OrganizationPlanEffectiveSummary {
  const summary = emptyMockOrganizationEffectiveSummary();
  for (const item of items) {
    const readiness = mockOrganizationEffectiveReadiness(item);
    if (readiness === "ready") summary.ready += 1;
    else if (readiness === "reviewed") summary.reviewed += 1;
    else if (readiness === "requires-decision") summary.pendingReview += 1;
    else summary.blocked += 1;
  }
  return summary;
}

function mockOrganizationSummary(items: OrganizationPlanItem[]): OrganizationPlan["summary"] {
  const summary = emptyMockOrganizationSummary();
  for (const item of items) {
    if (item.decision === "undecided") summary.undecided += 1;
    if (item.decision === "accepted") summary.accepted += 1;
    if (item.decision === "kept") summary.kept += 1;
    if (item.decision === "edited") summary.edited += 1;
    if (item.validity === "needs_analysis") summary.needsAnalysis += 1;
    if (item.validity === "needs_review") summary.needsReview += 1;
    if (item.validity === "needs_review" && item.decision === "undecided") summary.pendingReview += 1;
    if (item.validity === "needs_review" && ["accepted", "edited", "kept"].includes(item.decision)) summary.reviewed += 1;
    if (item.validity === "ready") summary.ready += 1;
    if (item.validity === "blocked") summary.blocked += 1;
    if (item.validity === "stale") summary.stale += 1;
    if (item.validity === "executing") summary.executing += 1;
    if (item.validity === "executed") summary.executed += 1;
    if (item.validity === "failed") summary.failed += 1;
    if (item.validity === "skipped") summary.skipped += 1;
    if (["accepted", "edited"].includes(item.decision) && ["ready", "needs_review"].includes(item.validity)) {
      summary.remainingExecutable += 1;
    }
  }
  return summary;
}

function defaultFileLibraryQueryForMock(marker: string): FileQueryRequestV2["query"] {
  return {
    scope: { kind: "all_enabled_roots" },
    text: marker,
    filters: {
      fileTypes: [], purposes: [], lifecycles: [], risks: [],
      sizeMin: null, sizeMax: null, modifiedFrom: null, modifiedTo: null,
      createdFrom: null, createdTo: null, duplicate: "any", review: "any",
      tagsAllOf: [], tagsAnyOf: [], tagsNoneOf: []
    },
    sort: { kind: "modified", direction: "desc" }
  };
}

function invalidMockSavedViewReferences(query: FileQueryRequestV2["query"] | undefined) {
  if (!query) return ["query_missing"];
  const tagIds = [...query.filters.tagsAllOf, ...query.filters.tagsAnyOf, ...query.filters.tagsNoneOf];
  return tagIds.filter((id, index, all) => !mockUserTags.some((tag) => tag.id === id) && all.indexOf(id) === index);
}

function mockLibraryFingerprint(query: FileQueryRequestV2["query"] | undefined) {
  const value = JSON.stringify(query ?? null);
  let digest = "";
  for (let lane = 0; lane < 8; lane += 1) {
    let hash = (2166136261 ^ Math.imul(lane + 1, 0x9e3779b9)) >>> 0;
    for (let index = 0; index < value.length; index += 1) {
      hash = Math.imul(hash ^ value.charCodeAt(index), 16777619);
    }
    digest += (hash >>> 0).toString(16).padStart(8, "0");
  }
  return digest;
}

function encodeMockLibraryCursor(cursor: { fingerprint: string; revision: number; offset: number }) {
  return `browser-library-cursor:${encodeURIComponent(JSON.stringify(cursor))}`;
}

function decodeMockLibraryCursor(value: string | null) {
  if (!value) return null;
  if (!value.startsWith("browser-library-cursor:")) throw new Error("library_cursor_invalid");
  try {
    return JSON.parse(decodeURIComponent(value.slice("browser-library-cursor:".length))) as { fingerprint: string; revision: number; offset: number };
  } catch {
    throw new Error("library_cursor_invalid");
  }
}

function mockFileTags(fileId: string) {
  const ids = mockFileTagIds.get(fileId) ?? new Set<string>();
  return [...ids]
    .map((id) => mockUserTags.find((tag) => tag.id === id))
    .filter((tag): tag is UserTag => Boolean(tag))
    .slice(0, 3)
    .map((tag) => ({ id: tag.id, displayName: tag.displayName, colorToken: tag.colorToken }));
}

function mockFileTimestamp(value: string) {
  return Math.floor(Date.parse(value) / 1000) || 0;
}

function searchMockFiles(query: string, limit: number): FileRecord[] {
  const normalized = query.trim().toLowerCase();
  if (!normalized) return [];
  return mockFiles
    .filter((item) => `${item.name} ${item.path} ${item.purpose}`.toLowerCase().includes(normalized))
    .slice(0, limit);
}

function mockActivateSearchResult(request?: Record<string, unknown>) {
  // Browser preview accepts the same wire DTO but deliberately does not
  // pretend to show/hide a native search window or mutate native navigation.
  const settingsTarget = request?.settingsTarget;
  if (settingsTarget !== undefined
    && settingsTarget !== null
    && !["search-scope", "global-index", "appearance", "ai"].includes(String(settingsTarget))) {
    throw new Error("search_navigation_settings_target_invalid");
  }
  return undefined;
}

function searchMockGlobalEntries(request?: GlobalSearchRequest): GlobalSearchResponse {
  return mockGlobalSearchResponseForTests(request);
}

/**
 * Keeps the browser preview on the same wire contract as the native global
 * search command. The optional arguments are intentionally exported for
 * behavior tests that need to exercise source-health states without mutating
 * the shared preview fixture.
 */
export function mockGlobalSearchResponseForTests(
  request?: GlobalSearchRequest,
  sourceHealth: GlobalSearchSourceHealth[] = defaultMockGlobalSourceHealth(),
  entries: GlobalSearchResult[] = mockGlobalEntries
): GlobalSearchResponse {
  const query = request?.query ?? "";
  const limit = request?.limit ?? 80;
  const offset = request?.cursor ? Number(request.cursor) : (request?.offset ?? 0);
  const normalized = query.trim().toLowerCase();
  const hasEnabledSources = sourceHealth.some((source) => source.enabled);
  const results = normalized && hasEnabledSources
    ? entries
    .filter((entry) => `${entry.name} ${entry.path} ${entry.extension}`.toLowerCase().includes(normalized))
    .slice(offset, offset + limit)
    : [];
  const indexStatus = hasEnabledSources
    ? mockGlobalIndexStatus()
    : {
      ...mockGlobalIndexStatus(),
      enabled: false,
      status: "unavailable",
      collectionComplete: false,
      indexedVolumes: 0,
      readyVolumes: 0,
      pendingVolumes: 0,
      processedEntries: 0,
      totalEntries: 0,
      lastSyncAt: null
    };
  return {
    version: 2,
    requestId: request?.requestId ?? "browser-request",
    normalizedQuery: query.trim(),
    results,
    indexStatus,
    collectionComplete: hasEnabledSources && indexStatus.collectionComplete,
    resultState: results.length ? "complete" : hasEnabledSources ? "empty" : "no_source",
    sourceRevision: `browser-source-revision-${hasEnabledSources ? "1" : "0"}`,
    sourceHealth
  };
}

function defaultMockGlobalSourceHealth(): GlobalSearchSourceHealth[] {
  return [{
    sourceId: "mock-volume",
    enabled: true,
    provider: "recursive_fallback",
    status: "ready",
    lastError: null,
    updatedAt: Date.parse(now) / 1000
  }];
}

function nextMockSearchWindowState(phase: SearchWindowSnapshot["phase"]): SearchWindowSnapshot {
  return {
    ...mockSearchWindowState,
    revision: mockSearchWindowState.revision + 1,
    phase
  };
}

function mockGlobalIndexStatus(): GlobalIndexStatus {
  return {
    platform: "browser",
    enabled: true,
    status: "ready",
    providerStatus: "browser_preview",
    processedEntries: mockGlobalEntries.length,
    collectionComplete: true,
    totalEntries: mockGlobalEntries.length,
    indexedVolumes: 1,
    readyVolumes: 1,
    pendingVolumes: 0,
    lastSyncAt: Date.parse(now) / 1000,
    lastError: null
  };
}

function mockGlobalIndexSources(): GlobalIndexSource[] {
  return [{
    volume: {
      id: "mock-volume",
      platform: "browser",
      stableVolumeId: "mock-volume",
      displayName: "Browser preview",
      mountPath: "C:/Users/Zen",
      filesystemType: "mock",
      driveKind: "fixed",
      enabled: true,
      provider: "recursive_fallback",
      indexStatus: "ready",
      lastError: null,
      journalId: null,
      journalCursor: null,
      lastFullIndexAt: Date.parse(now) / 1000,
      lastIncrementalSyncAt: Date.parse(now) / 1000,
      entryCount: mockGlobalEntries.length,
      createdAt: Date.parse(now) / 1000,
      updatedAt: Date.parse(now) / 1000
    },
    canPause: true,
    canRebuild: true,
    technicalDetail: null
  }];
}

function addMockManagedScope(request?: AddManagedScopeRequest): ManagedScope {
  const scope: ManagedScope = {
    id: `mock-scope-${mockManagedScopeState.length + 1}`,
    path: String(request?.path ?? "C:/Users/Zen/Documents"),
    globalEntryId: request?.globalEntryId ?? null,
    enabled: request?.enabled ?? true,
    allowLocalAi: request?.allowLocalAi ?? true,
    allowCloudAi: request?.allowCloudAi ?? false,
    createdAt: Date.parse(now) / 1000,
    updatedAt: Date.parse(now) / 1000
  };
  mockManagedScopeState = [...mockManagedScopeState, scope];
  return scope;
}

function updateMockManagedScope(request?: UpdateManagedScopePolicyRequest): ManagedScope {
  const index = mockManagedScopeState.findIndex((scope) => scope.id === request?.id);
  if (index < 0) throw new Error("managed scope not found");
  const current = mockManagedScopeState[index];
  const updated = {
    ...current,
    ...(request?.enabled === undefined ? {} : { enabled: request.enabled }),
    ...(request?.allowLocalAi === undefined ? {} : { allowLocalAi: request.allowLocalAi }),
    ...(request?.allowCloudAi === undefined ? {} : { allowCloudAi: request.allowCloudAi }),
    updatedAt: Date.parse(now) / 1000
  };
  mockManagedScopeState = mockManagedScopeState.map((scope, itemIndex) => itemIndex === index ? updated : scope);
  return updated;
}

function mockAiManagementStatus(): AiManagementStatus {
  const enabledScopes = mockManagedScopeState.filter((scope) => scope.enabled);
  return {
    enabledScopeCount: enabledScopes.length,
    managedEntryCount: enabledScopes.length,
    pendingJobCount: 0,
    runningJobCount: 0,
    cloudScopeCount: enabledScopes.filter((scope) => scope.allowCloudAi).length,
    policySummary: enabledScopes.some((scope) => scope.allowCloudAi)
      ? "managed_scope_only_cloud_enabled"
      : "managed_scope_only_cloud_disabled"
  };
}

function applyLibraryFilter(files: FileRecord[], filter?: FileLibraryFilters["libraryFilter"]): FileRecord[] {
  if (!filter || filter === "all") return files;
  if (filter === "active") return files.filter((item) => item.lifecycle === "Active");
  if (filter === "archive") return files.filter((item) => item.lifecycle === "Archive");
  if (filter === "review") return files.filter((item) => item.requires_confirmation);
  if (filter === "duplicate") return files.filter((item) => item.is_duplicate);
  if (filter === "sensitive") return files.filter((item) => item.risk_level === "Sensitive" || item.lifecycle === "Sensitive");
  return files;
}

function mockStats(): DashboardStats {
  const totalSize = mockFiles.reduce((sum, item) => sum + item.size, 0);
  return {
    totalFiles: mockFiles.length,
    totalSize,
    diskTotalSize: 512 * 1024 ** 3,
    diskFreeSize: 210 * 1024 ** 3,
    diskUsageRatio: 0.59,
    duplicateFiles: mockFiles.filter((item) => item.is_duplicate).length,
    largeFiles: mockFiles.filter((item) => item.size > 50 * 1024 ** 2).length,
    sensitiveFiles: mockFiles.filter((item) => item.risk_level === "Sensitive").length,
    needsConfirmation: mockFiles.filter((item) => item.requires_confirmation).length,
    byType: countBy(mockFiles, "file_type"),
    byLifecycle: countBy(mockFiles, "lifecycle"),
    lastScannedAt: now
  };
}

function mockOperationPreviews(args?: Record<string, unknown>): OperationPreviewResult {
  const limit = Number(args?.limit ?? 1000);
  const offset = Number(args?.offset ?? 0);
  const previews: OperationPreview[] = mockFiles
    .filter((item) => item.suggested_action !== "Keep" || item.requires_confirmation)
    .map((item, index) => ({
      id: `preview-${item.id}`,
      fileId: item.id,
      file_id: item.id,
      operation_type: "move",
      source_path: item.path,
      target_path: `C:/Users/Zen/${item.lifecycle}/${item.name}`,
      old_name: item.name,
      new_name: item.name,
      status: "pending",
      risk_level: item.risk_level,
      confidence: item.confidence,
      requires_confirmation: item.requires_confirmation,
      suggested_action: item.suggested_action,
      is_duplicate: item.is_duplicate,
      reason: "Browser mock preview",
      selected_by_default: index === 0,
      is_executable: true,
      editable_new_name: true,
      target_parent_exists: true,
      will_create_parent: false,
      operationFingerprint: `browser-preview-fingerprint-${item.id}`
    }));

  return {
    previews: previews.slice(offset, offset + limit),
    total: previews.length,
    limit,
    offset,
    truncated: false,
    hasMore: offset + limit < previews.length
  };
}

function mockStorageAnalysis(): StorageAnalysis {
  const candidates = [
    {
      id: "storage-safe-node-modules",
      path: "C:/Users/Zen/Projects/demo/node_modules",
      name: "node_modules",
      size: 1_850_000_000,
      tier: "Safe",
      category: "Regenerable development output",
      reason: "Build output or dependency cache can usually be recreated.",
      suggested_action: "MoveToTrash",
      risk_note: "Review project context first: dependency folders can contain linked packages or local patches.",
      trash_allowed: true,
      selected_by_default: true
    },
    {
      id: "storage-safe-build",
      path: "C:/Users/Zen/Projects/demo/build",
      name: "build",
      size: 640_000_000,
      tier: "Safe",
      category: "Regenerable development output",
      reason: "Build output can usually be recreated.",
      suggested_action: "MoveToTrash",
      risk_note: "Confirm this is generated output before adding it to the cleanup list.",
      trash_allowed: true,
      selected_by_default: false
    },
    {
      id: "storage-review-download",
      path: "C:/Users/Zen/Downloads/course-video.mp4",
      name: "course-video.mp4",
      size: 780_000_000,
      tier: "Review",
      category: "Downloads",
      reason: "User-owned content needs review before cleanup.",
      suggested_action: "Reveal",
      risk_note: "Open the location and review it manually.",
      trash_allowed: false,
      selected_by_default: false
    },
    {
      id: "storage-caution-app",
      path: "C:/Program Files/Example",
      name: "Example",
      size: 2_400_000_000,
      tier: "Caution",
      category: "Application",
      reason: "Use the app uninstaller instead of deleting files directly.",
      suggested_action: "UninstallAdvice",
      risk_note: "Manual deletion can leave services and shared components behind.",
      trash_allowed: false,
      selected_by_default: false
    }
  ] satisfies StorageAnalysis["candidates"];

  return {
    total_size: candidates.reduce((sum, candidate) => sum + candidate.size, 0),
    reclaimable_estimate: candidates
      .filter((candidate) => candidate.tier === "Safe" && candidate.trash_allowed)
      .reduce((sum, candidate) => sum + candidate.size, 0),
    review_estimate: candidates
      .filter((candidate) => candidate.tier === "Review")
      .reduce((sum, candidate) => sum + candidate.size, 0),
    candidates,
    denied_paths: [],
    warnings: []
  };
}

let mockOperationLogState: OperationLog[] | null = null;

function mockOperationLogs(): OperationLog[] {
  if (mockOperationLogState) return mockOperationLogState;
  const makeLog = (overrides: Partial<OperationLog>): OperationLog => ({
    id: "history-default",
    batch_id: "history-batch-a",
    operation_type: "move",
    source_path: "C:/Users/Zen/Documents/example.txt",
    target_path: "C:/Users/Zen/Documents/Organized/example.txt",
    old_name: "example.txt",
    new_name: "example.txt",
    status: "success",
    error_message: null,
    created_at: now,
    can_undo: true,
    path_before: "C:/Users/Zen/Documents/example.txt",
    path_after: "C:/Users/Zen/Documents/Organized/example.txt",
    name_before: "example.txt",
    name_after: "example.txt",
    can_restore: true,
    restored_at: null,
    restore_status: "not_restored",
    restore_error: null,
    ...overrides
  });
  mockOperationLogState = [
    makeLog({
      id: "history-a-restored",
      batch_id: "history-batch-a",
      old_name: "brief-final.pdf",
      new_name: "brief-final.pdf",
      source_path: "C:/Users/Zen/Documents/brief-final.pdf",
      target_path: "C:/Users/Zen/Documents/Work/brief-final.pdf",
      path_before: "C:/Users/Zen/Documents/brief-final.pdf",
      path_after: "C:/Users/Zen/Documents/Work/brief-final.pdf",
      restore_status: "restored",
      restored_at: "2026-07-06T09:08:00.000Z",
      can_restore: false
    }),
    makeLog({
      id: "history-a-restorable",
      batch_id: "history-batch-a",
      old_name: "brief-draft.pdf",
      new_name: "brief-draft.pdf",
      source_path: "C:/Users/Zen/Documents/brief-draft.pdf",
      target_path: "C:/Users/Zen/Documents/Work/brief-draft.pdf",
      path_before: "C:/Users/Zen/Documents/brief-draft.pdf",
      path_after: "C:/Users/Zen/Documents/Work/brief-draft.pdf"
    }),
    makeLog({
      id: "history-a-manual-recovery",
      batch_id: "history-batch-a",
      old_name: "brief-recovery.pdf",
      new_name: "brief-recovery.pdf",
      source_path: "C:/Users/Zen/Documents/brief-recovery.pdf",
      target_path: "C:/Users/Zen/Documents/Work/brief-recovery.pdf",
      path_before: "C:/Users/Zen/Documents/brief-recovery.pdf",
      path_after: "C:/Users/Zen/Documents/Work/brief-recovery.pdf",
      status: "manual_review",
      can_restore: false,
      restore_status: "manual_review",
      restore_phase: "target_committed",
      restore_error: "target_committed_durability_unknown: preserve the recovery item and review both paths",
      restore_claim_path: "C:/Users/Zen/Documents/.zen-canvas-claim-history-a-manual-recovery"
    }),
    makeLog({
      id: "history-a-failed",
      batch_id: "history-batch-a",
      status: "failed",
      old_name: "brief-locked.pdf",
      new_name: "brief-locked.pdf",
      can_restore: false,
      restore_status: "failed",
      restore_error: "The previous operation failed before it created a restorable journal entry."
    }),
    makeLog({
      id: "history-a-skipped",
      batch_id: "history-batch-a",
      status: "skipped",
      old_name: "brief-skipped.pdf",
      new_name: "brief-skipped.pdf",
      can_restore: false,
      restore_status: "unavailable",
      restore_error: "The item was skipped and has no restore source."
    }),
    makeLog({
      id: "history-b-restorable",
      batch_id: "history-batch-b",
      old_name: "photo-2026.png",
      new_name: "photo-2026.png",
      source_path: "C:/Users/Zen/Pictures/photo-2026.png",
      target_path: "C:/Users/Zen/Pictures/Archive/photo-2026.png",
      path_before: "C:/Users/Zen/Pictures/photo-2026.png",
      path_after: "C:/Users/Zen/Pictures/Archive/photo-2026.png"
    }),
    makeLog({
      id: "history-b-missing",
      batch_id: "history-batch-b",
      old_name: "missing-source.docx",
      new_name: "missing-source.docx",
      path_before: "",
      path_after: "",
      can_restore: false,
      restore_status: "unavailable",
      restore_error: "The source file is missing from the restore journal."
    }),
    makeLog({
      id: "history-b-canceled",
      batch_id: "history-batch-b",
      old_name: "canceled-upload.zip",
      new_name: "canceled-upload.zip",
      can_restore: false,
      restore_status: "canceled",
      restore_error: "Restore was canceled before this item was processed."
    }),
    makeLog({
      id: "history-c-restorable",
      batch_id: "history-batch-c",
      old_name: "design-system.fig",
      new_name: "design-system.fig",
      source_path: "C:/Users/Zen/Projects/design-system.fig",
      target_path: "C:/Users/Zen/Projects/Archive/design-system.fig",
      path_before: "C:/Users/Zen/Projects/design-system.fig",
      path_after: "C:/Users/Zen/Projects/Archive/design-system.fig"
    })
  ];
  return mockOperationLogState;
}

function mockRestoreMoves(args?: Record<string, unknown>): RestoreMovesResult {
  const ids = Array.isArray((args?.request as Record<string, unknown> | undefined)?.logIds)
    ? ((args?.request as Record<string, unknown>).logIds as unknown[]).map(String)
    : [];
  const source = mockOperationLogs();
  const logs = ids
    .map((id) => source.find((log) => log.id === id))
    .filter((log): log is OperationLog => Boolean(log))
    .map((log) => {
      const outcome: OperationLog["restore_status"] = log.id === "history-a-restorable"
        ? "restored"
        : log.id === "history-b-restorable"
          ? "failed"
          : "canceled";
      return {
        ...log,
        can_restore: false,
        restored_at: outcome === "restored" ? now : null,
        restore_status: outcome,
        restore_error: outcome === "failed" ? "The destination path is occupied by another file." : outcome === "canceled" ? "Restore was canceled before this item was processed." : null
      };
    });
  mockOperationLogState = source.map((log) => logs.find((updated) => updated.id === log.id) ?? log);
  return {
    logs,
    restored: logs.filter((log) => log.restore_status === "restored").length,
    failed: logs.filter((log) => log.restore_status === "failed").length
  };
}

function mockResolveOperationRecovery(args?: Record<string, unknown>): RecoveryActionResult {
  const request = args?.request as { logId?: string; action?: string; targetPath?: string | null } | undefined;
  const id = String(request?.logId ?? "");
  const action = request?.action === "delete" ? "delete" : request?.action === "move" ? "move" : "keep_both";
  const source = mockOperationLogs();
  const original = source.find((log) => log.id === id);
  if (!original?.restore_claim_path) throw new Error("recovery_claim_missing");
  const operationType = action === "delete" ? "permanent_delete" : action === "move" ? "move" : "copy";
  const target = action === "delete"
    ? "Permanent deletion quarantine"
    : request?.targetPath || "C:/Users/Zen/Documents/brief-recovery (recovered).pdf";
  const actionLog: OperationLog = {
    ...original,
    id: `browser-recovery-action-${Date.now()}`,
    batch_id: `browser-recovery-batch-${Date.now()}`,
    operation_type: operationType,
    source_path: original.restore_claim_path,
    target_path: target,
    path_before: original.restore_claim_path,
    path_after: target,
    old_name: original.old_name,
    new_name: original.new_name,
    status: "success",
    can_undo: operationType === "move",
    can_restore: operationType === "move",
    restore_status: operationType === "move" ? "not_restored" : "unavailable",
    restore_error: null,
    restore_phase: "idle"
  };
  const updatedOriginal: OperationLog = {
    ...original,
    restore_error: action === "delete" ? "recovery_action_delete_completed" : `recovery_action_${action === "move" ? "move" : "keep_both"}_completed:${target}`,
    restore_claim_path: action === "move" ? target : action === "delete" ? null : original.restore_claim_path,
    restore_phase: "manual_review"
  };
  mockOperationLogState = [actionLog, ...source.map((log) => log.id === id ? updatedOriginal : log)];
  return {
    original_log: updatedOriginal,
    action_log: actionLog,
    target_path: action === "delete" ? null : target
  };
}

function mockStorageCleanupStatus(jobId: string): StorageCleanupScanStatus {
  return {
    jobId,
    status: "completed",
    progress: {
      jobId,
      scannedEntries: 48,
      currentPath: "C:/Users/Zen/Projects/demo/node_modules",
      totalSize: mockStorageAnalysis().total_size
    },
    analysis: mockStorageAnalysis(),
    error: null,
    startedAt: Date.now().toString(),
    completedAt: Date.now().toString()
  };
}

function mockCleanupExecutionResult(args?: Record<string, unknown>): CleanupExecutionResult {
  const ids = new Set(cleanupSelectionIds(args));
  const logs: CleanupExecutionResult["logs"] = mockStorageAnalysis()
    .candidates
    .filter((candidate) => ids.has(candidate.id))
    .map((candidate) => {
      const allowed =
        candidate.tier === "Safe" &&
        candidate.trash_allowed &&
        candidate.suggested_action === "MoveToTrash";
      return {
        path: candidate.path,
        name: candidate.name,
        size: candidate.size,
        status: allowed ? "success" : "skipped",
        message: allowed
          ? "Moved to Zen Canvas Safe Trash. Restore it from Recovery records."
          : "Only safe cleanup candidates can be moved."
      };
    });

  return {
    moved: logs.filter((log) => log.status === "success").length,
    skipped: logs.filter((log) => log.status === "skipped").length,
    failed: 0,
    logs
  };
}

function mockSafeTrashExecutionResult(args?: Record<string, unknown>): CleanupExecutionResult {
  const result = mockCleanupExecutionResult(args);
  return {
    ...result,
    logs: result.logs.map((log, index) => ({
      ...log,
      message: log.status === "success"
        ? "Moved to Zen Canvas Safe Trash. Restore it from Recovery records."
        : log.message,
      itemId: `browser-cleanup-item-${index}`,
      trashPath: `C:/Users/Zen/.zen-canvas-trash/items/browser-cleanup-batch/item-${index}/${log.name}`
    }))
  };
}

function mockAnalyzeCleanupCandidatesWithAI(args?: Record<string, unknown>): StorageCandidate[] {
  if (mockCleanupPresentationScenario() === "request-unavailable") {
    throw new Error("provider_request_unavailable: browser presentation fixture only");
  }
  const requested = new Set(
    Array.isArray(args?.ids)
      ? args.ids.filter((id): id is string => typeof id === "string")
      : []
  );
  return mockStorageAnalysis().candidates
    .filter((candidate) => requested.has(candidate.id))
    .map((candidate) => {
      if (candidate.tier === "Safe") {
        return {
          ...candidate,
          reason: `AI 风险说明：${candidate.reason}`,
          risk_note: candidate.risk_note
            ? `AI 分析后建议：${candidate.risk_note}`
            : "AI 分析后建议：清理前确认没有本地补丁或未提交依赖改动。"
        };
      }
      return {
        ...candidate,
        selected_by_default: false,
        reason: `AI 风险说明：${candidate.reason}`,
        risk_note: candidate.risk_note
          ? `AI 分析后建议：${candidate.risk_note}`
          : "AI 分析后建议：保持人工确认。"
      };
    });
}

function mockCleanupTrashBatches(): CleanupTrashBatch[] {
  const movedAt = mockCleanupCreatedAt;
  const items: CleanupTrashBatch["items"] = [
    {
      id: "browser-cleanup-restorable-0",
      batchId: "browser-cleanup-batch",
      originalPath: "C:/Users/Zen/Projects/demo/node_modules",
      trashPath: "C:/Users/Zen/.zen-canvas-trash/items/browser-cleanup-batch/item-0/node_modules",
      name: "node_modules",
      size: 1_850_000_000,
      movedAt,
      restoredAt: null,
      status: "moved",
      message: "Moved to Zen Canvas Safe Trash."
    },
    {
      id: "browser-cleanup-restorable-1",
      batchId: "browser-cleanup-batch",
      originalPath: "C:/Users/Zen/Projects/demo/dist",
      trashPath: "C:/Users/Zen/.zen-canvas-trash/items/browser-cleanup-batch/item-1/dist",
      name: "dist",
      size: 120_000_000,
      movedAt,
      restoredAt: null,
      status: "moved",
      message: "Moved to Zen Canvas Safe Trash."
    },
    {
      id: "browser-cleanup-conflict",
      batchId: "browser-cleanup-batch",
      originalPath: "C:/Users/Zen/Projects/demo/cache",
      trashPath: "C:/Users/Zen/.zen-canvas-trash/items/browser-cleanup-batch/item-2/cache",
      name: "cache",
      size: 12_000_000,
      movedAt,
      restoredAt: null,
      status: "moved",
      message: "Restore blocked because the original path already exists."
    },
    {
      id: "browser-cleanup-missing",
      batchId: "browser-cleanup-batch",
      originalPath: "C:/Users/Zen/Projects/demo/temp",
      trashPath: "C:/Users/Zen/.zen-canvas-trash/items/browser-cleanup-batch/item-3/temp",
      name: "temp",
      size: 8_000_000,
      movedAt,
      restoredAt: null,
      status: "missing",
      message: "Safe trash path is missing."
    },
    {
      id: "browser-cleanup-restored",
      batchId: "browser-cleanup-batch",
      originalPath: "C:/Users/Zen/Projects/demo/old-build",
      trashPath: "C:/Users/Zen/.zen-canvas-trash/items/browser-cleanup-batch/item-4/old-build",
      name: "old-build",
      size: 14_000_000,
      movedAt,
      restoredAt: "2026-07-06T09:12:00.000Z",
      status: "restored",
      message: "Restored from Zen Canvas Safe Trash."
    }
  ];
  return [
    {
      id: "browser-cleanup-batch",
      createdAt: movedAt,
      root: "C:/Users/Zen/Projects/demo",
      totalItems: 5,
      totalSize: 2_004_000_000,
      status: "success",
      items: items.map((item) => ({
        ...item,
        ...(mockCleanupRestoreState.get(item.id) ?? {})
      }))
    }
  ];
}

function mockCleanupRestorePreviewItem(item: CleanupTrashBatch["items"][number]): CleanupRestorePreview["items"][number] {
  const blockingReason = item.id === "browser-cleanup-conflict"
    ? "conflict"
    : item.id === "browser-cleanup-missing"
      ? "missing"
      : item.status === "restored"
        ? "already restored"
        : item.status === "moved"
          ? null
          : item.status;
  return {
    ...item,
    canRestore: blockingReason === null,
    blockingReason
  };
}

function mockCleanupRestoreResult(args?: Record<string, unknown>): CleanupRestoreResult {
  const ids = Array.isArray(args?.itemIds) ? args.itemIds.map(String) : [];
  const logs = ids.map((itemId) => {
    const item = mockCleanupTrashBatches()[0]?.items.find((candidate) => candidate.id === itemId);
    const status: CleanupRestoreResult["logs"][number]["status"] = item?.id === "browser-cleanup-conflict"
      ? "conflict"
      : item?.status === "missing"
        ? "missing"
        : item?.id === "browser-cleanup-restorable-1"
          ? "failed"
          : item?.status === "moved"
            ? "restored"
            : "failed";
    if (item && status === "restored") {
      mockCleanupRestoreState.set(item.id, {
        status: "restored",
        restoredAt: new Date().toISOString(),
        message: "Restored from Zen Canvas Safe Trash."
      });
    }
    return {
      itemId,
      originalPath: item?.originalPath ?? "",
      trashPath: item?.trashPath ?? "",
      status,
      message: status === "conflict"
        ? "The destination path is occupied by another file."
        : status === "missing"
          ? "The safe trash source is missing."
          : status === "failed"
            ? "Restore failed."
            : "Restored from Zen Canvas Safe Trash."
    };
  });
  return {
    restored: logs.filter((log) => log.status === "restored").length,
    conflicts: logs.filter((log) => log.status === "conflict").length,
    missing: logs.filter((log) => log.status === "missing").length,
    failed: logs.filter((log) => log.status === "failed").length,
    canceled: 0,
    logs
  };
}

function mockCleanupPreviewCandidates(args?: Record<string, unknown>): CleanupPreviewItem[] {
  const ids = new Set(cleanupSelectionIds(args));
  return mockStorageAnalysis()
    .candidates
    .filter((candidate) => ids.has(candidate.id))
    .filter((candidate) => candidate.tier === "Safe" && candidate.trash_allowed)
    .map((candidate) => ({
      id: `cleanup-preview-${candidate.id}`,
      candidate_id: candidate.id,
      path: candidate.path,
      name: candidate.name,
      size: candidate.size,
      tier: candidate.tier,
      category: candidate.category,
      reason: candidate.reason,
      operation_type: "move_to_trash_preview",
      target_path: "Recycle Bin",
      status: "pending",
      requires_confirmation: true,
      is_executable: false,
      blocking_reason: "Browser mock preview only"
    }));
}

function mockCleanupPreviewOperations(args?: Record<string, unknown>): OperationPreviewResult {
  const ids = new Set(cleanupSelectionIds(args));
  const previews: OperationPreview[] = mockStorageAnalysis()
    .candidates
    .filter((candidate) => ids.has(candidate.id))
    .filter((candidate) => candidate.tier === "Safe" && candidate.trash_allowed)
    .map((candidate) => {
      const finding = mockAnalysisFindings.find((item) => item.id === candidate.id);
      const name = finding?.pathSnapshot?.split(/[\\/]/).filter(Boolean).pop() ?? candidate.name;
      return {
        id: `cleanup-trash-${candidate.id}`,
        fileId: candidate.id,
        operation_type: "move_to_trash",
        source_path: finding?.pathSnapshot ?? candidate.path,
        target_path: "Recycle Bin",
        old_name: name,
        new_name: name,
        status: "pending",
        risk_level: "Normal",
        confidence: 1,
        requires_confirmation: true,
        suggested_action: "DeleteCandidate",
        is_duplicate: false,
        reason: finding?.reason ?? candidate.reason,
        selected_by_default: true,
        is_executable: true,
        editable_new_name: false,
        target_parent_exists: true,
        will_create_parent: false
      };
    });

  if (mockCleanupPresentationScenario() === "request-unavailable") {
    const reviewSelections = new Map<string, number>();
    if (Array.isArray(args?.selections)) {
      for (const selection of args.selections) {
        if (!selection || typeof selection !== "object") continue;
        const candidate = selection as { findingId?: unknown; reviewConfirmation?: { decisionRevision?: unknown } };
        if (typeof candidate.findingId === "string" && typeof candidate.reviewConfirmation?.decisionRevision === "number") {
          reviewSelections.set(candidate.findingId, candidate.reviewConfirmation.decisionRevision);
        }
      }
    }
    for (const finding of mockAnalysisFindings) {
      if (!ids.has(finding.id) || finding.tier !== "review" || finding.decision !== "acknowledged") continue;
      if (reviewSelections.get(finding.id) !== finding.decisionRevision) continue;
      const name = finding.pathSnapshot?.split(/[\\/]/).filter(Boolean).pop() ?? finding.title;
      previews.push({
        id: `browser-presentation-only-preview:${finding.id}`,
        fileId: finding.id,
        operation_type: "move_to_trash",
        source_path: finding.pathSnapshot ?? "",
        target_path: "Recycle Bin",
        old_name: name,
        new_name: name,
        status: "pending",
        risk_level: "Normal",
        confidence: 1,
        requires_confirmation: true,
        suggested_action: "DeleteCandidate",
        is_duplicate: false,
        reason: "Browser presentation fixture only; backend authority is not represented.",
        selected_by_default: true,
        is_executable: true,
        editable_new_name: false,
        target_parent_exists: true,
        will_create_parent: false
      });
    }
  }

  return {
    previews,
    total: previews.length,
    limit: previews.length,
    offset: 0,
    truncated: false,
    hasMore: false
  };
}

function cleanupSelectionIds(args?: Record<string, unknown>): string[] {
  if (!Array.isArray(args?.selections)) return [];
  return args.selections
    .map((selection) => {
      if (!selection || typeof selection !== "object") return null;
      const value = (selection as { findingId?: unknown }).findingId;
      return typeof value === "string" ? value : null;
    })
    .filter((value): value is string => value !== null);
}

function mockSettings(settings?: AppSettings): AppSettings {
  if (settings) return settings;
  const onboardingFixture = new URLSearchParams(globalThis.location?.search ?? "").get("pm01-onboarding");
  const hasPresentationFolder = onboardingFixture === "folder-index-on" || onboardingFixture === "folder-index-off";
  return {
    closeBehavior: "ask",
    folderNamingLanguage: "en",
    defaultScanFolders: hasPresentationFolder ? [{
      id: "browser-presentation-only-onboarding-root",
      path: "C:/Presentation/Zen Documents",
      label: "Zen Documents",
      enabled: true,
      createdAt: "2026-09-28T00:00:00.000Z"
    }] : [],
    restoreRetentionDays: 30,
    launchAtLogin: false,
    backgroundIndexOnStartup: onboardingFixture !== "folder-index-off",
    searchHotkey: DEFAULT_SEARCH_HOTKEY,
    searchScopeMode: "all",
    customSearchRoots: [],
    organizeRootMode: "current_folder",
    organizeRootPath: undefined,
    useLegacyBuiltinClassificationRules: false,
    useLearnedRulesAsAutoRules: false
  };
}

let persistedMockSettings: AppSettings | undefined;
let mockSettingsRevision = 0;

function getMockVersionedSettings(): VersionedAppSettings {
  return {
    settings: persistedMockSettings ?? mockSettings(),
    revision: mockSettingsRevision
  };
}

function saveMockVersionedSettings(request: SaveSettingsRequest): VersionedAppSettings {
  if (request.expectedRevision !== mockSettingsRevision) {
    throw new Error("settings_revision_conflict");
  }
  persistedMockSettings = request.settings;
  mockSettingsRevision += 1;
  return getMockVersionedSettings();
}

let mockAISettingsState: AISettings | null = null;
let mockApiKeyConfigured = false;

// Explicit URL-selected browser fixtures exercise the renderer only. They are
// not derived from persisted AI settings and do not represent backend authority.
function browserPresentationFeatureReadinessFixture(): AIProductFeatureReadiness {
  const fixture = new URLSearchParams(globalThis.location?.search ?? "").get("pm01-readiness") ?? "disconnected";
  let providerState: AIReadinessState = "disabled";
  let providerReason = "provider_disabled";
  let providerMode: "local" | "cloud" = "cloud";
  if (fixture === "invalid-provider") {
    providerState = "needs_provider";
    providerReason = "provider_configuration_invalid";
  } else if (fixture === "missing-credential") {
    providerState = "needs_credential";
    providerReason = "provider_credential_missing";
  } else if (fixture !== "disconnected") {
    providerState = "ready";
    providerReason = "provider_configuration_ready";
  }
  if (fixture.includes("local")) providerMode = "local";

  const provider: AIProductFeatureReadiness["provider"] = {
    state: providerState,
    reason: providerReason,
    providerMode,
    providerKind: providerMode === "local" ? "ollama" : "openai_compatible",
    providerPreset: null,
    model: providerState === "ready" ? "browser-presentation-only" : null,
    credentialRequired: providerMode === "cloud",
    credentialConfigured: providerState === "ready" && providerMode === "cloud",
    settingsRevision: "browser-presentation-fixture",
    bindingFingerprint: "browser-presentation-fixture"
  };
  const disclosure = {
    providerPayloadIncludesFileName: false,
    providerPayloadIncludesParentPath: false,
    providerPayloadIncludesFullPath: false,
    providerPayloadIncludesFileContent: false,
    providerContentIsBounded: true
  };
  const managedState: AIReadinessState = fixture === "scope-missing"
    ? "scope_missing"
    : fixture === "managed-consent-missing"
      ? "needs_consent"
      : providerState;
  const managedReason = fixture === "scope-missing"
    ? "managed_scope_missing"
    : fixture === "managed-consent-missing"
      ? "managed_local_ai_consent_required"
      : providerReason;
  const managedScopes: AIProductFeatureReadiness["managedScopes"] = fixture === "scope-missing"
    ? []
    : [{
      state: managedState,
      reason: managedReason,
      provider,
      managedScopeId: "browser-presentation-scope",
      scopeFingerprint: "browser-presentation-scope",
      bindingFingerprint: "browser-presentation-fixture",
      disclosure
    }];

  let cleanupState: AIReadinessState = providerState;
  let cleanupReason = providerReason;
  if (providerState === "ready") {
    cleanupState = "ready";
    cleanupReason = "cleanup_ai_ready";
    if (fixture === "cleanup-disabled") {
      cleanupState = "disabled";
      cleanupReason = "cleanup_ai_disabled";
    } else if (fixture === "cleanup-local-consent-missing") {
      cleanupState = "needs_consent";
      cleanupReason = "cleanup_local_ai_consent_required";
    } else if (fixture === "cleanup-cloud-consent-missing") {
      cleanupState = "needs_consent";
      cleanupReason = "cleanup_cloud_ai_consent_required";
    }
  }
  return {
    provider,
    managedScopes,
    cleanup: {
      state: cleanupState,
      reason: cleanupReason,
      provider,
      cleanupAiEnabled: cleanupState !== "disabled",
      localAiAllowed: cleanupState === "ready" && providerMode === "local",
      cloudAiAllowed: cleanupState === "ready" && providerMode === "cloud",
      bindingFingerprint: "browser-presentation-fixture",
      disclosure
    }
  };
}

function listMockOrganizationPlans(): OrganizationPlan[] {
  const fixture = new URLSearchParams(globalThis.location?.search ?? "").get("pm01-organize");
  if (fixture && ["needs-analysis", "ready", "review", "blocked"].includes(fixture)
    && !mockOrganizationPlans.some((plan) => plan.id.startsWith("browser-presentation-organization-plan-"))) {
    const fileIds = mockFiles.filter((file) => !file.is_stale).slice(0, 3).map((file) => file.id);
    createMockOrganizationPlan({
      title: "Browser presentation fixture",
      source: { kind: "explicit", fileIds },
      expectedCount: fileIds.length
    });
  }
  return mockOrganizationPlans;
}

function mockAISettings(settings?: AISettings): AISettings {
  if (settings) {
    const action = settings.apiKeyAction ?? (settings.apiKey.trim() ? "replace" : "preserve");
    if (action === "replace") {
      if (!settings.apiKey.trim()) throw new Error("Replacing the AI API key requires a non-empty value.");
      mockApiKeyConfigured = true;
    } else if (action === "clear") {
      mockApiKeyConfigured = false;
    }
    mockAISettingsState = {
      ...settings,
      apiKey: "",
      apiKeyAction: "preserve",
      apiKeyConfigured: mockApiKeyConfigured
    };
    return { ...mockAISettingsState };
  }
  if (mockAISettingsState) return { ...mockAISettingsState };
  return {
    enabled: false,
    provider: "openai_compatible",
    preset: "deepseek",
    baseUrl: "https://api.deepseek.com",
    chatPath: "/chat/completions",
    modelsPath: "/models",
    apiKey: "",
    apiKeyAction: "preserve",
    apiKeyConfigured: mockApiKeyConfigured,
    model: "deepseek-v4-flash",
    temperature: 0,
    maxTokens: 8192,
    batchSize: 10,
    classificationConcurrency: 2,
    timeoutSeconds: 120,
    sendFullPath: false,
    sendParentPath: true,
    classificationMode: "ai_first",
    cleanupAiEnabled: true,
    cleanupLocalAiAllowed: false,
    cleanupCloudAiAllowed: false,
    forceJsonOutput: true,
    enableThinking: false,
    reasoningEffort: null,
    extraBodyJson: null,
    diagnosticsMode: "off",
    includeSensitiveDocumentContentInDiagnostics: false
  };
}

function mockAIProviderPresets(): AIProviderPreset[] {
  return [
    ["deepseek", "DeepSeek — Recommended", "https://api.deepseek.com", "deepseek-v4-flash", true, true, true],
    ["kimi", "Kimi / Moonshot", "https://api.moonshot.ai/v1", "kimi-k2.6", true, true, false],
    ["qwen_dashscope", "Qwen / DashScope", "https://dashscope.aliyuncs.com/compatible-mode/v1", "qwen-plus", true, true, false],
    ["zhipu_glm", "智谱 GLM", "https://open.bigmodel.cn/api/paas/v4", "glm-5.2", true, true, true],
    ["doubao_ark", "豆包 / 火山方舟", "https://ark.cn-beijing.volces.com/api/v3", "", false, false, false],
    ["minimax", "MiniMax", "https://api.minimaxi.com/v1", "MiniMax-M2.5", true, true, false],
    ["hunyuan", "腾讯混元", "https://api.hunyuan.cloud.tencent.com/v1", "hunyuan-turbos", false, false, false],
    ["baidu_qianfan", "百度千帆", "https://qianfan.baidubce.com/v2", "ernie-4.5-turbo-32k", false, false, false],
    ["siliconflow", "SiliconFlow / 硅基流动", "https://api.siliconflow.cn/v1", "", false, false, false],
    ["baichuan", "百川智能（兼容入口）", "", "", false, false, false],
    ["stepfun", "阶跃星辰 / StepFun", "https://api.stepfun.com/v1", "step-3.5-flash", false, false, false],
    ["yi", "零一万物 / Yi（兼容平台）", "", "", false, false, false],
    ["custom_openai_compatible", "自定义 OpenAI-compatible", "", "", false, false, false],
    ["ollama", "Ollama — Local model", "http://localhost:11434", "qwen3:8b", false, true, false]
  ].map(([id, label, defaultBaseUrl, defaultModel, supportsResponseFormat, supportsThinking, supportsReasoningEffort]) => ({
    id: id as AIProviderPreset["id"],
    label: String(label),
    providerKind: id === "ollama" ? "ollama" : "openai_compatible",
    defaultBaseUrl: String(defaultBaseUrl),
    defaultChatPath: id === "ollama" ? "/api/chat" : "/chat/completions",
    modelsPath: id === "ollama" ? "/api/tags" : "/models",
    defaultModel: String(defaultModel),
    suggestedModels: [String(defaultModel)].filter(Boolean),
    supportsResponseFormat: Boolean(supportsResponseFormat),
    supportsJsonMode: true,
    supportsThinking: Boolean(supportsThinking),
    supportsReasoningEffort: Boolean(supportsReasoningEffort)
  }));
}

function mockAIModels(): AIModelInfo[] {
  return mockAIProviderPresets().flatMap((preset) =>
    (preset.suggestedModels ?? []).map((id) => ({ id, ownedBy: preset.id, discovered: false }))
  );
}

let mockAITraceState: AIRequestTrace[] = [];

function mockAITraces(): AIRequestTrace[] {
  return mockAITraceState.map((trace) => ({ ...trace }));
}

function mockAIConnectionTest(settings?: AISettings): AIConnectionTestResult {
  const resolved = settings ? { ...settings } : mockAISettings();
  return {
    ok: true,
    message: "{\"ok\":true}",
    model: resolved.model,
    provider: resolved.provider,
    preset: resolved.preset,
    elapsedMs: 32
  };
}

function mockAIDebugClassification(args?: Record<string, unknown>): AIDebugClassificationResult {
  const settings = mockAISettings();
  const fileId = String(args?.target ?? args?.fileId ?? "mock-report");
  const rawResponsePreview = JSON.stringify({
    choices: [
      {
        finish_reason: "stop",
        message: {
          role: "assistant",
          content: JSON.stringify({
            classifications: [
              {
                refId: "f1",
                fileType: "Document",
                purpose: "Work",
                lifecycle: "Active",
                riskLevel: "Normal",
                suggestedAction: "Move",
                targetTemplate: "Work/Reports",
                confidence: 0.86,
                reason: "Browser mock debug response."
              }
            ]
          })
        }
      }
    ]
  }, null, 2);

  return {
    provider: settings.provider,
    preset: settings.preset,
    model: settings.model,
    baseUrl: settings.baseUrl,
    chatPath: settings.chatPath,
    forceJsonOutput: settings.forceJsonOutput,
    enableThinking: settings.enableThinking,
    maxTokens: settings.maxTokens,
    batchSize: settings.batchSize,
    requestUsedResponseFormat: settings.forceJsonOutput && settings.preset !== "ollama" && settings.preset !== "doubao_ark" && settings.preset !== "hunyuan" && settings.preset !== "siliconflow" && settings.preset !== "baidu_qianfan" && settings.preset !== "baichuan" && settings.preset !== "stepfun" && settings.preset !== "yi",
    requestUsedThinkingField: "disabled",
    httpStatus: 200,
    providerResponseSummary: "has_choices=true; choice_count=1; finish_reason=stop; message_keys=[content,role]; content_type=string; content_length=180; has_reasoning_content=false; reasoning_content_length=0",
    rawResponsePreview,
    messageContentPreview: "{\"classifications\":[{\"refId\":\"f1\",\"fileType\":\"Document\"}]}",
    reasoningContentPreview: "",
    extractedContentPreview: "{\"classifications\":[{\"refId\":\"f1\",\"fileType\":\"Document\"}]}",
    cleanedContentPreview: "{\"classifications\":[{\"refId\":\"f1\",\"fileType\":\"Document\"}]}",
    parseStage: "parse_ai_classification_response",
    parseError: null,
    success: true,
    refId: "f1",
    realFileId: fileId,
    path: "C:/Users/Zen/Documents/project-report.pdf",
    modelReturnedRefId: "f1",
    modelReturnedId: null,
    idMappingMatched: true,
    missingOptionalFields: ["suggestedName", "keywords", "context", "requiresConfirmation"],
    fallbackApplied: true,
    itemParseWarnings: ["requiresConfirmation missing; safe fallback applied"]
  };
}

function mockAIClassifyFiles(args?: Record<string, unknown>): RuleExecutionSummary {
  const options = args?.options as {
    pendingOnly?: boolean;
    onlyUnclassified?: boolean;
    onlyLowConfidence?: boolean;
    limit?: number;
    force?: boolean;
    allowOverwriteUserCorrections?: boolean;
  } | null | undefined;
  const limit = Math.max(1, Number(options?.limit ?? mockFiles.length));
  const candidates = mockFiles
    .filter((file) => !options?.pendingOnly || (
      file.classification_status !== "classified"
      || file.confidence < 0.65
      || file.requires_confirmation
    ))
    .filter((file) => !options?.onlyUnclassified || file.classification_status !== "classified")
    .filter((file) => !options?.onlyLowConfidence || file.confidence < 0.65)
    .filter((file) => {
      const protectedByUser = file.matched_rules.some((rule) =>
        rule === "user_correction"
        || rule === "user_confirmed"
        || rule === "manual"
        || rule.startsWith("learned:")
      );
      if (protectedByUser && !options?.allowOverwriteUserCorrections) return false;
      if (options?.force) return true;
      return !(
        file.classification_status === "classified"
        && !file.requires_confirmation
        && file.matched_rules.some((rule) => rule.startsWith("ai:"))
      );
    })
    .slice(0, limit);
  return applyMockAIClassification(candidates);
}

function mockAIClassifySelectedFiles(args?: Record<string, unknown>): RuleExecutionSummary {
  const ids = new Set(Array.isArray(args?.fileIds) ? args.fileIds.map(String) : []);
  return applyMockAIClassification(mockFiles.filter((file) => ids.has(file.id)));
}

function mockCorrectClassification(args?: Record<string, unknown>): void {
  const fileId = String(args?.fileId ?? "");
  const correction = args?.correction as ClassificationCorrectionRequest | undefined;
  const file = mockFiles.find((item) => item.id === fileId);
  if (!file || !correction) return;
  file.file_type = correction.fileType;
  file.purpose = correction.purpose;
  file.lifecycle = correction.lifecycle;
  file.context = correction.context;
  file.risk_level = correction.riskLevel;
  file.suggested_action = correction.suggestedAction;
  file.suggested_target_path = correction.targetTemplate;
  file.suggested_name = correction.suggestedName ?? "";
  file.classification_reason = correction.reason || "User corrected classification.";
  file.classification_status = "classified";
  file.matched_rules = ["learned:browser-mock"];
  file.confidence = 1;
  file.requires_confirmation = correction.riskLevel === "Sensitive" || correction.suggestedAction === "Review";
}

function applyMockAIClassification(files: FileRecord[]): RuleExecutionSummary {
  for (const file of files) {
    file.classification_status = "classified";
    file.classification_reason = "AI browser mock classified this file from metadata only.";
    file.matched_rules = ["ai:browser-mock:model"];
    file.confidence = Math.max(file.confidence, 0.82);
    if (file.purpose === "Unknown") file.purpose = "Work";
    if (file.suggested_action === "Keep") {
      file.suggested_action = "Move";
      file.suggested_target_path = `${file.directory}/ZenCanvas/${file.file_type}`;
    }
    file.requires_confirmation = file.requires_confirmation || file.confidence < 0.65 || file.risk_level === "Sensitive";
  }
  return {
    scanned: files.length,
    updated: files.length,
    skipped: 0,
    needsConfirmation: files.filter((file) => file.requires_confirmation).length
  };
}

function file(overrides: Partial<FileRecord>): FileRecord {
  return {
    id: "mock-file",
    name: "file.txt",
    path: "C:/Users/Zen/Documents/file.txt",
    directory: "C:/Users/Zen/Documents",
    extension: "txt",
    size: 1024,
    file_type: "Document",
    purpose: "Unknown",
    lifecycle: "Inbox",
    context: "Browser mock",
    risk_level: "Normal",
    hash: null,
    created_at: now,
    modified_at: now,
    scanned_at: now,
    last_seen_at: now,
    is_hidden: false,
    is_deleted: false,
    is_duplicate: false,
    suggested_action: "Keep",
    suggested_target_path: "",
    suggested_name: "",
    confidence: 0.5,
    classification_reason: "Browser mock data",
    classification_status: "classified",
    matched_rules: [],
    requires_confirmation: false,
    ...overrides
  };
}

function countBy<T extends keyof FileRecord>(files: FileRecord[], key: T): Record<string, number> {
  return files.reduce<Record<string, number>>((counts, item) => {
    const value = String(item[key]);
    counts[value] = (counts[value] ?? 0) + 1;
    return counts;
  }, {});
}

const automationMock = createAutomationMock({
  roots: mockManagedScanRoots,
  fingerprint: mockLibraryFingerprint,
  snapshotRevision: () => mockLibraryRevision,
  createPlan: (intent, revision, runId) => createMockOrganizationPlan({ title: intent.title, source: { kind: "all_matching", query: intent.scopeQuery, queryFingerprint: intent.scopeFingerprint, snapshotRevision: revision, excludedFileIds: [] } }, runId)
});
