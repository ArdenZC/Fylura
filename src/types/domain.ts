export type FileType =
  | "Document"
  | "Image"
  | "Video"
  | "Audio"
  | "Code"
  | "ArchivePackage"
  | "Installer"
  | "Spreadsheet"
  | "Presentation"
  | "Other";

export type Purpose =
  | "Project"
  | "Teaching"
  | "Study"
  | "Work"
  | "Personal"
  | "Career"
  | "Finance"
  | "Identity"
  | "Media"
  | "Installer"
  | "Temporary"
  | "Archive"
  | "Document"
  | "Duplicate Review"
  | "Unknown";

export type Lifecycle =
  | "Inbox"
  | "Active"
  | "Reference"
  | "Archive"
  | "Disposable"
  | "Duplicate"
  | "Sensitive"
  | "TrashReview"
  | "Unknown";

export type RiskLevel = "Normal" | "Sensitive" | "System" | "Caution" | "Unknown";

export type SuggestedAction =
  | "Keep"
  | "Rename"
  | "Move"
  | "MoveAndRename"
  | "Archive"
  | "Copy"
  | "Duplicate"
  | "Replace"
  | "Review"
  | "DeleteCandidate"
  | "Unknown";

export type DispatchZone = "CoreAssets" | "QuietArchive" | "PrivacyVault" | "CleanupLane";
export type SearchSourceType = "user_space" | "folder" | "cloud" | "external";
export type RestoreStatus =
  | "not_restored"
  | "pending"
  | "restored"
  | "failed"
  | "unavailable"
  | "canceled"
  | "manual_review";
export type RestorePhase =
  | "idle"
  | "prepared"
  | "source_claimed"
  | "copying"
  | "target_committed"
  | "source_cleanup_pending"
  | "completed"
  | "rolled_back"
  | "manual_review";
export type CleanupTier = "Safe" | "Review" | "Caution";
export type CleanupActionKind = "MoveToTrash" | "Reveal" | "UninstallAdvice" | "AppInternalCleanup" | "None";
export type OperationType =
  | "move"
  | "rename"
  | "move_rename"
  | "copy"
  | "duplicate"
  | "replace"
  | "permanent_delete"
  | "move_to_trash"
  | "create_directory";
export type ClassificationStatus = "unclassified" | "classified";
export type FolderNamingLanguage = "en" | "zh";
export type CloseBehavior = "ask" | "minimize" | "quit";
export type RestoreRetentionDays = 15 | 30 | 60 | 90;
export type RuleExecutionMode = "inbox_only" | "all_changed_or_rule_changed";
export type SearchScopeMode = "all" | "current_scan" | "custom_roots";
export type OrganizeRootMode = "current_folder" | "zen_canvas_folder" | "custom_root";
export type AIProviderKind = "openai_compatible" | "ollama";
export type AIReadinessState =
  | "ready"
  | "disabled"
  | "needs_provider"
  | "needs_credential"
  | "needs_consent"
  | "scope_missing"
  | "scope_disabled"
  | "policy_blocked"
  | "temporarily_unavailable"
  | "error";
export type AIProviderMode = "local" | "cloud";

export interface AIDataDisclosure {
  providerPayloadIncludesFileName: boolean;
  providerPayloadIncludesParentPath: boolean;
  providerPayloadIncludesFullPath: boolean;
  providerPayloadIncludesFileContent: boolean;
  providerContentIsBounded: boolean;
}

export interface AIProviderReadiness {
  state: AIReadinessState;
  reason: string;
  providerMode: AIProviderMode | null;
  providerKind: AIProviderKind | null;
  providerPreset: AIProviderPresetId | null;
  model: string | null;
  credentialRequired: boolean;
  credentialConfigured: boolean;
  settingsRevision: string | null;
  bindingFingerprint: string;
}

export interface ManagedAIReadiness {
  state: AIReadinessState;
  reason: string;
  provider: AIProviderReadiness;
  managedScopeId: string;
  scopeFingerprint: string | null;
  bindingFingerprint: string;
  disclosure: AIDataDisclosure;
}

export interface CleanupAIReadiness {
  state: AIReadinessState;
  reason: string;
  provider: AIProviderReadiness;
  cleanupAiEnabled: boolean;
  localAiAllowed: boolean;
  cloudAiAllowed: boolean;
  bindingFingerprint: string;
  disclosure: AIDataDisclosure;
}

export interface AIProductFeatureReadiness {
  provider: AIProviderReadiness;
  managedScopes: ManagedAIReadiness[];
  cleanup: CleanupAIReadiness;
}
export type AIProviderPresetId =
  | "deepseek"
  | "kimi"
  | "qwen_dashscope"
  | "zhipu_glm"
  | "minimax"
  | "baichuan"
  | "doubao_ark"
  | "siliconflow"
  | "hunyuan"
  | "baidu_qianfan"
  | "stepfun"
  | "yi"
  | "custom_openai_compatible"
  | "ollama";
export type AIClassificationMode = "ai_first" | "rules_first" | "hybrid";
export type AIAuthKind = "none" | "bearer_api_key" | "api_key_header" | "qianfan_ak_sk";
export type AITraceMode = "off" | "failures" | "all";

export interface AIProviderCapabilities {
  supportsModelDiscovery: boolean;
  supportsResponseFormatJsonObject: boolean;
  supportsJsonSchema: boolean;
  supportsThinking: boolean;
  supportsThinkingToggle: boolean;
  supportsReasoningEffort: boolean;
  supportsUsage: boolean;
  supportsStreaming: boolean;
}

export interface AIParameterProfile {
  temperatureMin: number;
  temperatureMax: number;
  defaultTemperature: number;
  maxOutputTokens: number | null;
  tokenParameter: "max_tokens" | "max_completion_tokens";
  thinkingStrategy: string;
}

export interface AIEndpointVariant {
  id: string;
  label: string;
  baseUrl: string;
}

export interface AICustomProviderProfile {
  id: string;
  name: string;
  baseUrl: string;
  chatPath: string;
  modelsPath?: string | null;
  model: string;
  supportsResponseFormat: boolean;
  supportsThinking: boolean;
  thinkingParameter: string;
  tokenParameter: string;
  contentPath: string;
  reasoningPath: string;
  temperatureMin: number;
  temperatureMax: number;
  maxOutputTokens: number;
  extraBodyJson?: string | null;
  apiKeyConfigured?: boolean;
}

export interface AIProviderPreset {
  id: AIProviderPresetId;
  label: string;
  providerKind: AIProviderKind;
  defaultBaseUrl: string;
  defaultChatPath: string;
  modelsPath?: string | null;
  defaultModel: string;
  suggestedModels?: string[];
  apiKeyEnvHint?: string;
  authKind?: AIAuthKind;
  capabilities?: AIProviderCapabilities;
  parameterProfile?: AIParameterProfile;
  endpointVariants?: AIEndpointVariant[];
  supportsResponseFormat: boolean;
  supportsJsonMode?: boolean;
  supportsThinking: boolean;
  supportsReasoningEffort: boolean;
  extraBodyStrategy?: string;
  docsUrl?: string | null;
}

export interface AISettings {
  enabled: boolean;
  provider: AIProviderKind;
  preset: AIProviderPresetId;
  baseUrl: string;
  chatPath: string;
  modelsPath?: string | null;
  apiKey: string;
  apiKeyAction?: "preserve" | "replace" | "clear";
  apiKeyConfigured?: boolean;
  model: string;
  temperature: number;
  maxTokens: number;
  batchSize: number;
  classificationConcurrency: number;
  timeoutSeconds: number;
  sendFullPath: boolean;
  sendParentPath: boolean;
  classificationMode: AIClassificationMode;
  cleanupAiEnabled: boolean;
  cleanupLocalAiAllowed: boolean;
  cleanupCloudAiAllowed: boolean;
  forceJsonOutput: boolean;
  enableThinking: boolean;
  reasoningEffort: string | null;
  extraBodyJson: string | null;
  diagnosticsMode?: AITraceMode;
  includeSensitiveDocumentContentInDiagnostics?: boolean;
  customProfiles?: AICustomProviderProfile[];
  activeCustomProfileId?: string | null;
}

export interface AIModelInfo {
  id: string;
  ownedBy?: string | null;
  discovered: boolean;
}

export interface AITraceUsage {
  promptTokens?: number | null;
  completionTokens?: number | null;
  totalTokens?: number | null;
}

export interface AIRequestTrace {
  traceId: string;
  jobId?: string | null;
  batchId?: string | null;
  startedAt: string;
  elapsedMs: number;
  operation: "connection_test" | "file_classification" | "cleanup_analysis" | "model_discovery" | "rule_proposal_generation" | "content_understanding";
  providerId: string;
  providerLabel: string;
  model: string;
  request: {
    urlHost: string;
    path: string;
    messageCount: number;
    targetCount?: number | null;
    batchSize?: number | null;
    maxTokens?: number | null;
    temperature?: number | null;
    forceJson: boolean;
    responseFormat?: string | null;
    thinkingMode?: string | null;
    extraBodyKeys: string[];
  };
  response: {
    httpStatus?: number | null;
    finishReason?: string | null;
    messageKeys: string[];
    contentType?: string | null;
    contentLength?: number | null;
    reasoningContentLength?: number | null;
    usage?: AITraceUsage | null;
  };
  rawProviderResponse?: string | null;
  extractedContent?: string | null;
  cleanedJsonText?: string | null;
  parsedJson?: unknown;
  parseStage: string;
  errorCode?: string | null;
  errorMessage?: string | null;
  truncated: boolean;
}

export interface AIConnectionTestResult {
  ok: boolean;
  message: string;
  model: string | null;
  provider: AIProviderKind | null;
  preset: AIProviderPresetId | null;
  elapsedMs: number;
}

export type CapabilityStatus =
  | "implemented"
  | "runtimeDependent"
  | "unavailable"
  | "needsMaterialization"
  | "needsPermission"
  | "providerOffline"
  | "filesystemInsufficient"
  | "notFixtureValidated";

export interface CapabilityLayers {
  platformFeatureAvailability: CapabilityStatus;
  runtimeEnvironmentCapability: CapabilityStatus;
  operationEligibility: CapabilityStatus;
}

export interface RuntimeCapabilities {
  platform?: string;
  architecture?: string;
  macosVersion?: string | null;
  aiDebugAvailable: boolean;
  realAIClassificationAvailable: boolean;
  credentialStoreAvailable: boolean;
  fileMutationAvailable: boolean;
  fileMutationUnavailableCode: string | null;
  copyAvailable: boolean;
  duplicateAvailable: boolean;
  renameAvailable: boolean;
  sameVolumeMoveAvailable: boolean;
  crossVolumeMoveAvailable: boolean;
  replaceAvailable: boolean;
  safeTrashAvailable: boolean;
  restoreAvailable: boolean;
  permanentDeleteAvailable: boolean;
  secureRemovalAvailable: boolean;
  packageMutationAvailable: boolean;
  iCloudMutationAvailable: boolean;
  fileProviderMutationAvailable: boolean;
  externalVolumeMutationAvailable: boolean;
  networkVolumeMutationAvailable: boolean;
  backendWatcherReconciliation: boolean;
  macosNativeSemanticsAvailable: boolean;
  macosSameVolumeMutationAvailable: boolean;
  macosRenameAvailable: boolean;
  macosSafeTrashAvailable: boolean;
  macosCloudMutationAvailable: boolean;
  macosFileProviderMutationAvailable: boolean;
  macosPackageMutationAvailable: boolean;
  macosCrossVolumeMutationAvailable: boolean;
  macosLifecycleAvailable: boolean;
  macosFinderAvailable: boolean;
  macosQuickLookThumbnailAvailable: boolean;
  macosQuickLookPreviewAvailable: boolean;
  macosRestoreAvailable: boolean;
  macosActivityPolicyAvailable: boolean;
  macosICloudAwarenessAvailable: boolean;
  macosFileProviderAwarenessAvailable: boolean;
  macosPackageAwarenessAvailable: boolean;
  fileProviderCapabilities: CapabilityLayers;
  externalVolumeCapabilities: CapabilityLayers;
  networkVolumeCapabilities: CapabilityLayers;
}

export interface AIDebugClassificationResult {
  provider: AIProviderKind;
  preset: AIProviderPresetId;
  model: string;
  baseUrl: string;
  chatPath: string;
  forceJsonOutput: boolean;
  enableThinking: boolean;
  maxTokens: number;
  batchSize: number;
  requestUsedResponseFormat: boolean;
  requestUsedThinkingField: string | null;
  httpStatus: number;
  providerResponseSummary: string;
  rawResponsePreview: string;
  messageContentPreview: string;
  reasoningContentPreview: string;
  extractedContentPreview: string;
  cleanedContentPreview: string;
  parseStage: string;
  parseError: string | null;
  success: boolean;
  refId: string;
  realFileId: string;
  path: string;
  modelReturnedRefId: string | null;
  modelReturnedId: string | null;
  idMappingMatched: boolean;
  missingOptionalFields: string[];
  fallbackApplied: boolean;
  itemParseWarnings: string[];
}

export interface AIClassificationProgressPayload {
  jobId: string;
  processed: number;
  total: number;
  batchIndex: number;
  batchCount: number;
  completedBatches: number;
  failedBatches: number;
  updated: number;
  skipped: number;
  needsConfirmation: number;
  stage: string;
  currentFilePreview: string;
  elapsedMs: number;
  estimatedRemainingMs?: number | null;
}

export interface RuleExecutionSummary {
  scanned: number;
  updated: number;
  skipped: number;
  needsConfirmation: number;
  failedBatches?: number;
  failedFiles?: number;
  warning?: string;
}

export interface ClassificationCorrectionRequest {
  fileType: FileType;
  purpose: Purpose;
  lifecycle: Lifecycle;
  context: string;
  riskLevel: RiskLevel;
  suggestedAction: SuggestedAction;
  targetTemplate: string;
  suggestedName?: string;
  reason?: string;
}

export interface ScanRootSetting {
  id: string;
  path: string;
  label: string;
  enabled: boolean;
  createdAt: string;
}

export interface SearchRootSetting {
  id: string;
  path: string;
  label: string;
  enabled: boolean;
  createdAt: string;
}

export type LibraryScope =
  | { kind: "current_scan"; roots: string[]; scanSessionId?: string }
  | { kind: "roots"; roots: string[] }
  | { kind: "all" };

export type DedupeScopeRequest = {
  kind: "allManagedFileLibrary" | "explicitEnabledScanRoots";
  rootIds?: string[];
};

export interface StartDedupeRunRequest {
  scope: DedupeScopeRequest;
  requestKey?: string | null;
  parentScanSessionId?: string | null;
}

export interface DedupeRun {
  id: string;
  requestKey: string;
  requestAttempt: number;
  parentScanSessionId: string | null;
  scope: Record<string, unknown>;
  scopeSnapshot: unknown;
  scopeHash: string;
  scopeSnapshotHash: string;
  publicationMode: "authoritative" | "diagnostic" | string;
  status: string;
  phase: string;
  revision: number;
  cancelRequested: boolean;
  rerunRequired: boolean;
  candidateFiles: number;
  candidatePhysicalObjects: number;
  candidateBytes: number;
  identityVerifiedFiles: number;
  identityUnknownFiles: number;
  hardlinkAliases: number;
  prehashedFiles: number;
  prehashPrunedFiles: number;
  fullHashedFiles: number;
  duplicateGroups: number;
  duplicateMembers: number;
  exactReclaimableBytes: number;
  potentialReclaimableBytes: number;
  processedFiles: number;
  processedBytes: number;
  totalBytes: number;
  warningCount: number;
  errorCount: number;
  startedAt: number | null;
  finishedAt: number | null;
  lastCheckpointAt: number | null;
  createdAt: number;
  updatedAt: number;
  errorCode: string | null;
  errorMessage: string | null;
}

export type AnalysisScopeRequest = {
  kind: "allManagedFileLibrary" | "explicitEnabledScanRoots" | "approvedCleanupPaths";
  rootIds?: string[];
  paths?: string[];
};

export interface StartAnalysisRunRequest {
  scope: AnalysisScopeRequest;
  detectorIds?: string[];
  requestKey?: string | null;
}

export interface AnalysisRun {
  id: string;
  requestKey: string;
  requestAttempt: number;
  scope: Record<string, unknown>;
  scopeHash: string;
  sourceSnapshot: unknown;
  sourceSnapshotHash: string;
  detectorSet: string[];
  detectorSetHash: string;
  status: string;
  phase: "preparing" | "running_detectors" | "finalizing" | "completed" | string;
  revision: number;
  cancelRequested: boolean;
  rerunRequired: boolean;
  detectorsTotal: number;
  detectorsCompleted: number;
  detectorsFailed: number;
  findingsStaged: number;
  findingsPublished: number;
  safeCount: number;
  reviewCount: number;
  cautionCount: number;
  exactReclaimableBytes: number;
  potentialReclaimableBytes: number;
  warningCount: number;
  errorCount: number;
  startedAt: number | null;
  finishedAt: number | null;
  lastCheckpointAt: number | null;
  createdAt: number;
  updatedAt: number;
  errorCode: string | null;
  errorMessage: string | null;
}

export interface AnalysisDetector {
  runId: string;
  detectorId: string;
  detectorVersion: number;
  status: string;
  revision: number;
  scannedSubjects: number;
  findingsStaged: number;
  findingsPublished: number;
  exactReclaimableBytes: number;
  potentialReclaimableBytes: number;
  startedAt: number | null;
  finishedAt: number | null;
  errorCode: string | null;
  errorMessage: string | null;
}

export interface AnalysisDetectorDescriptor {
  detectorId: string;
  version: number;
  title: string;
  description: string;
  supportsAllManagedScope: boolean;
  supportsApprovedPaths: boolean;
}

export interface AnalysisFinding {
  id: string;
  findingKey: string;
  runId: string;
  detectorId: string;
  detectorVersion: number;
  scopeHash: string;
  status: "staged" | "active" | "stale" | "superseded" | "discarded" | string;
  tier: "safe" | "review" | "caution" | string;
  category: string;
  actionKind: string;
  title: string;
  reason: string;
  riskNote: string | null;
  confidence: "exact" | "estimated" | "unknown" | string;
  sizeBytes: number;
  exactReclaimableBytes: number | null;
  potentialReclaimableBytes: number;
  requiresConfirmation: boolean;
  executable: boolean;
  primarySubjectKind: string;
  primarySubjectId: string;
  pathSnapshot: string | null;
  identitySnapshot: Record<string, unknown>;
  evidenceSummary: Record<string, unknown>;
  revision: number;
  createdAt: number;
  updatedAt: number;
  publishedAt: number | null;
  staleAt: number | null;
  decision: "open" | "acknowledged" | "dismissed" | "snoozed" | null;
  snoozedUntil: number | null;
  decisionRevision: number | null;
}

export interface AnalysisFindingEvidence {
  id: string;
  findingId: string;
  evidenceKind: string;
  subjectKind: string;
  subjectId: string | null;
  pathSnapshot: string | null;
  value: Record<string, unknown>;
  createdAt: number;
  isCurrentAssessment?: boolean;
}

export interface AnalysisFindingDecision {
  findingKey: string;
  decision: "open" | "acknowledged" | "dismissed" | "snoozed";
  snoozedUntil: number | null;
  note: string | null;
  revision: number;
  createdAt: number;
  updatedAt: number;
}

export interface AnalysisFindingPage {
  findings: AnalysisFinding[];
  nextCursor: string | null;
  limit: number;
}

export interface DedupeAuthority {
  revision: number;
  status: "healthy" | "rebuild_required" | "degraded" | string;
  lastAuthoritativeRunId: string | null;
  scopeHash: string;
  updatedAt: number;
}

export interface DedupeGroup {
  id: string;
  sizeEach: number;
  fullHash: string;
  fullHashAlgorithm: string;
  fullHashVersion: number;
  memberCount: number;
  physicalCopyCount: number;
  hardlinkAliasCount: number;
  exactReclaimableBytes: number | null;
  potentialReclaimableBytes: number;
  reclaimableConfidence: "exact" | "estimated" | "unknown" | string;
  status: string;
  lastBuiltRunId: string;
  revision: number;
  createdAt: number;
  updatedAt: number;
  lastVerifiedAt: number;
  representativePaths: string[];
}

export interface DedupeGroupMember {
  groupId: string;
  fileId: string;
  pathSnapshot: string;
  physicalKey: string | null;
  identityStatus: string;
  isHardlinkAlias: boolean;
  size: number;
  modifiedNs: number | null;
  verifiedAt: number;
}

export interface DedupeGroupPage {
  groups: DedupeGroup[];
  nextCursor: string | null;
  limit: number;
}

export type LibraryFilter = "all" | "active" | "archive" | "review" | "duplicate" | "sensitive";

export interface FileLibraryFilters {
  libraryFilter?: LibraryFilter;
}

export type LibraryMatchMode = "any" | "only" | "exclude";

export type FileLibraryScopeV2 =
  | { kind: "all_enabled_roots" }
  | { kind: "roots"; scanRootIds: string[] }
  | { kind: "current_scan"; scanSessionId: string };

export interface FileQueryFiltersV2 {
  fileTypes: FileType[];
  purposes: Purpose[];
  lifecycles: Lifecycle[];
  risks: RiskLevel[];
  sizeMin: number | null;
  sizeMax: number | null;
  modifiedFrom: number | null;
  modifiedTo: number | null;
  createdFrom: number | null;
  createdTo: number | null;
  duplicate: LibraryMatchMode;
  review: LibraryMatchMode;
  tagsAllOf: string[];
  tagsAnyOf: string[];
  tagsNoneOf: string[];
}

export type FileLibrarySortKind = "relevance" | "modified" | "created" | "name" | "size" | "confidence";
export type FileLibrarySortDirection = "asc" | "desc";

export interface FileLibrarySortV2 {
  kind: FileLibrarySortKind;
  direction: FileLibrarySortDirection;
}

export interface FileQuerySpecV2 {
  scope: FileLibraryScopeV2;
  text: string | null;
  filters: FileQueryFiltersV2;
  sort: FileLibrarySortV2;
}

export interface FileQueryRequestV2 {
  version: 2;
  requestId: string;
  query: FileQuerySpecV2;
  pageSize: number;
  cursor?: string | null;
}

export interface UserTagPreview {
  id: string;
  displayName: string;
  colorToken: string;
}

export interface FileLibraryNativeSemantics {
  isPackage: boolean;
  cloudBacking: "local" | "icloud" | "file_provider" | "unknown" | string;
  contentAvailability: "local" | "not_local" | "downloading" | "metadata_only" | "unknown" | string;
}

export interface FileLibrarySummary {
  id: string;
  name: string;
  extension: string;
  displayDirectory: string;
  size: number;
  modifiedAt: number;
  createdAt: number;
  isDirectory: boolean;
  fileType: string;
  purpose: string;
  lifecycle: string;
  risk: string;
  confidence: number;
  isDuplicate: boolean;
  requiresReview: boolean;
  isStale: boolean;
  tags: UserTagPreview[];
  tagCount: number;
  nativeSemantics?: FileLibraryNativeSemantics | null;
}

export interface LibraryScopeRootHealth {
  id: string;
  displayName: string;
  healthStatus: string;
  enabled: boolean;
  available: boolean;
  generation: number;
  message: string | null;
}

export interface LibraryScopeHealth {
  state: string;
  roots: LibraryScopeRootHealth[];
  invalidReferences: string[];
  message: string | null;
}

export interface FileQueryResponseV2 {
  version: 2;
  requestId: string;
  queryFingerprint: string;
  snapshotRevision: number;
  files: FileLibrarySummary[];
  totalCount: number | null;
  countState: "exact" | "deferred";
  countToken: string | null;
  nextCursor: string | null;
  hasMore: boolean;
  resultState: "complete" | "partial" | "empty" | "failed" | "snapshot_expired" | string;
  scopeHealth: LibraryScopeHealth;
}

export interface ResolveFileLibraryExactCountRequestV2 {
  version: 2;
  requestId: string;
  countToken: string;
}

export interface ResolveFileLibraryExactCountResponseV2 {
  version: 2;
  requestId: string;
  queryFingerprint: string;
  snapshotRevision: number;
  totalCount: number;
  countState: "exact";
}

export interface FileLibraryFindingSummary {
  id: string;
  findingType: string;
  severity: string;
  detector: string;
  state: string;
  decision: string;
  evidenceSummary: unknown;
  analysisRevision: number;
}

export interface FilePersonalMetadata {
  fileId: string;
  note: string;
  favorite: boolean;
  revision: number;
  editable: boolean;
}
export interface FilePersonalMetadataResponse {
  metadata: FilePersonalMetadata | null;
  items: { fileId: string; name: string; isStale: boolean }[];
  hasMore: boolean;
}
export type FilePersonalMetadataRequest =
  | { kind: "get"; fileId: string }
  | { kind: "update"; fileId: string; note: string; favorite: boolean; expectedRevision: number }
  | { kind: "list"; favorites: boolean; offset: number };

export interface FileLibraryDetail {
  id: string;
  name: string;
  path: string;
  directory: string;
  extension: string;
  size: number;
  modifiedAt: number;
  createdAt: number;
  isDirectory: boolean;
  fileType: string;
  purpose: string;
  lifecycle: string;
  context: string;
  risk: string;
  confidence: number;
  classificationStatus: string;
  classificationReason: string;
  matchedRules: string[];
  suggestedAction: string;
  suggestedTargetPath: string;
  suggestedName: string;
  isDuplicate: boolean;
  requiresReview: boolean;
  isStale: boolean;
  lastSeenAt: number;
  scanRootId: string | null;
  scanRootName: string | null;
  scopeHealth: string | null;
  duplicateGroupId: string | null;
  duplicateGroupSize: number;
  tags: UserTagPreview[];
  activeFindings: FileLibraryFindingSummary[];
  safeActions: string[];
  revision: number;
  contentStatus?: string;
  contentPolicy?: string;
  contentSummary?: string | null;
  contentKeywords?: string[];
  contentLanguage?: string | null;
  contentProvenance?: string | null;
  contentTruncated?: boolean | null;
  contentTextRetained?: boolean | null;
  contentRevision?: number | null;
  nativeSemantics?: FileLibraryNativeSemantics | null;
}

export interface LibraryTypeCount {
  fileType: string;
  count: number;
}

export interface FileLibrarySelectionSummary {
  count: number;
  totalSize: number;
  typeCounts: LibraryTypeCount[];
  missingCount: number;
  staleCount: number;
  excludedCount: number;
  commonDirectory: string | null;
  commonTags: UserTagPreview[];
  commonTagIds: string[];
  partialTagCommonalityCount: number;
  snapshotRevision: number;
  queryFingerprint: string | null;
}

export type LibrarySelectionV1 =
  | { kind: "explicit"; fileIds: string[] }
  | {
    kind: "all_matching";
    query: FileQuerySpecV2;
    queryFingerprint: string;
    snapshotRevision: number;
    excludedFileIds: string[];
  };

export interface MutateFileUserTagsRequest {
  selection: LibrarySelectionV1;
  tagIds: string[];
  operation: "add" | "remove";
  expectedCount?: number | null;
}

export interface MutateFileUserTagsResult {
  appliedCount: number;
  alreadyPresentCount: number;
  missingCount: number;
  excludedCount: number;
  revision: number;
}

export interface UserTag {
  id: string;
  displayName: string;
  colorToken: string;
  usageCount: number;
  createdAt: number;
  updatedAt: number;
  revision: number;
}

export interface CreateUserTagRequest {
  displayName: string;
  colorToken: string;
}

export interface UpdateUserTagRequest {
  id: string;
  displayName: string;
  colorToken: string;
  expectedRevision: number;
}

export interface DeleteUserTagRequest {
  id: string;
  confirm: boolean;
  expectedUsageCount: number;
  expectedRevision: number;
}

export interface LibrarySavedView {
  id: string;
  displayName: string;
  query: FileQuerySpecV2;
  queryFingerprint: string;
  position: number;
  createdAt: number;
  updatedAt: number;
  revision: number;
  invalidReferences: string[];
}

export interface CreateLibrarySavedViewRequest {
  displayName: string;
  query: FileQuerySpecV2;
  position?: number | null;
}

export interface UpdateLibrarySavedViewRequest {
  id: string;
  displayName: string;
  query: FileQuerySpecV2;
  position: number;
  expectedRevision: number;
}

export interface DeleteLibrarySavedViewRequest {
  id: string;
  expectedRevision: number;
}

export type OrganizationPlanStatus =
  | "draft"
  | "building"
  | "ready"
  | "stale"
  | "executing"
  | "partially_completed"
  | "completed"
  | "cancelled"
  | "failed";

export interface OrganizationPlan {
  id: string;
  title: string;
  status: OrganizationPlanStatus;
  sourceKind: "explicit" | "all_matching";
  sourceQueryFingerprint: string | null;
  sourceSnapshotRevision: number;
  requestedCount: number;
  materializedCount: number;
  plannerVersion: number;
  revision: number;
  activeExecutionId: string | null;
  activeOperationBatchId: string | null;
  lastErrorCode: string | null;
  lastErrorDetail: string | null;
  createdAt: number;
  updatedAt: number;
  readyAt: number | null;
  completedAt: number | null;
  summary: OrganizationPlanSummary;
  effectiveSummary: OrganizationPlanEffectiveSummary | null;
}

export interface OrganizationPlanSummary {
  undecided: number;
  accepted: number;
  kept: number;
  edited: number;
  needsAnalysis: number;
  needsReview: number;
  pendingReview: number;
  reviewed: number;
  ready: number;
  blocked: number;
  stale: number;
  executing: number;
  executed: number;
  failed: number;
  skipped: number;
  remainingExecutable: number;
}

export interface OrganizationPlanEffectiveSummary {
  ready: number;
  reviewed: number;
  pendingReview: number;
  blocked: number;
}

export interface OrganizationPlanItem {
  id: string;
  planId: string;
  ordinal: number;
  fileIdSnapshot: string;
  sourcePathSnapshot: string;
  sourceNameSnapshot: string;
  sourceSizeSnapshot: number;
  sourceMtimeSnapshot: number;
  sourceIsDirSnapshot: boolean;
  proposalFingerprint: string;
  semanticExplanation: OrganizationSemanticExplanation | null;
  proposalKind: "move" | "rename" | "move_rename" | "keep" | "blocked";
  proposedTargetDirectory: string;
  proposedName: string;
  proposedTargetPath: string;
  decision: "undecided" | "accepted" | "kept" | "edited";
  editedName: string | null;
  validity: "ready" | "needs_analysis" | "needs_review" | "blocked" | "stale" | "executing" | "executed" | "failed" | "skipped";
  reviewState: "ready" | "needs_review" | "reviewed" | "blocked" | "needs_analysis" | "stale" | "executing" | "executed" | "failed" | "skipped";
  effectiveReadiness: OrganizationPlanGroupReadiness;
  confidence: number;
  riskLevel: string;
  requiresConfirmation: boolean;
  blockingCode: string | null;
  blockingDetail: string | null;
  authoritativePreviewId: string | null;
  reviewReasons: string[];
  availableActions: string[];
  operationLogId: string | null;
  executionId: string | null;
  revision: number;
  createdAt: number;
  updatedAt: number;
}

export interface OrganizationSemanticExplanation {
  assessmentFingerprint: string;
  reason: string;
}

export interface OrganizationPlanItemPage {
  planId: string;
  planRevision: number;
  items: OrganizationPlanItem[];
  nextCursor: string | null;
  hasMore: boolean;
}

export type OrganizationPlanGroupReadiness = "ready" | "requires-decision" | "reviewed" | "blocked";

export interface OrganizationReviewReasonCount {
  reason: string;
  count: number;
}

export interface OrganizationPlanGroupSample {
  itemId: string;
  sourceName: string;
  sourcePath: string;
  proposedName: string;
  decision: OrganizationPlanItem["decision"];
  validity: OrganizationPlanItem["validity"];
}

export interface OrganizationPlanGroupSummary {
  groupId: string;
  planId: string;
  label: string;
  targetDirectory: string | null;
  proposalKind: OrganizationPlanItem["proposalKind"];
  readiness: OrganizationPlanGroupReadiness;
  riskLevel: string;
  itemCount: number;
  totalBytes: number;
  acceptedCount: number;
  excludedCount: number;
  staleCount: number;
  conflictCount: number;
  confidenceBand: string;
  reviewReasonCounts: OrganizationReviewReasonCount[];
  availableActions: string[];
  groupActions: {
    canAcceptAll: boolean;
    canKeepAll: boolean;
    canClearAll: boolean;
  };
  projectionFingerprint: string;
  sampleItems: OrganizationPlanGroupSample[];
  revision: number;
}

export interface OrganizationPlanGroupPage {
  planId: string;
  planRevision: number;
  groups: OrganizationPlanGroupSummary[];
  effectiveSummary: OrganizationPlanEffectiveSummary;
  projectionFingerprint: string;
  nextCursor: string | null;
  hasMore: boolean;
}

export interface OrganizationPlanGroupItemPage {
  planId: string;
  groupId: string;
  planRevision: number;
  projectionFingerprint: string;
  items: OrganizationPlanItem[];
  nextCursor: string | null;
  hasMore: boolean;
}

export interface UpdateOrganizationPlanGroupDecisionResult {
  plan: OrganizationPlan;
  group: OrganizationPlanGroupSummary | null;
}

export interface OrganizationDryRunItem {
  itemId: string;
  operationKind: string;
  from: string;
  to: string;
  editedFilename: string | null;
  parentDirectoryToCreate: string | null;
  collision: boolean;
  crossVolume: boolean;
  riskLevel: string;
  requiresConfirmation: boolean;
  sourceHealth: string;
  authoritativePreviewId: string | null;
  executable: boolean;
  blockingCode: string | null;
}

export interface OrganizationPlanDryRun {
  planId: string;
  planRevision: number;
  selectedCount: number;
  executableCount: number;
  blockedCount: number;
  staleCount: number;
  totalBytes: number;
  operationKinds: string[];
  items: OrganizationDryRunItem[];
  executionBatchLimit: number;
  dryRunFingerprint: string;
}

export type OrganizationPlanSelection =
  | { allAccepted: true; itemIds: [] }
  | { allAccepted: false; itemIds: [string, ...string[]] };

export interface ExecuteOrganizationPlanResult {
  plan: OrganizationPlan;
  executionId: string;
  operationBatchId: string;
  attemptedCount: number;
  succeededCount: number;
  failedCount: number;
  skippedCount: number;
}

export interface AppSettings {
  closeBehavior: CloseBehavior;
  folderNamingLanguage: FolderNamingLanguage;
  defaultScanFolders: ScanRootSetting[];
  restoreRetentionDays: RestoreRetentionDays;
  launchAtLogin: boolean;
  backgroundIndexOnStartup: boolean;
  searchHotkey: string;
  searchScopeMode: SearchScopeMode;
  customSearchRoots: SearchRootSetting[];
  organizeRootMode: OrganizeRootMode;
  organizeRootPath?: string | null;
  useLegacyBuiltinClassificationRules: boolean;
  useLearnedRulesAsAutoRules: boolean;
}

export interface VersionedAppSettings {
  settings: AppSettings;
  revision: number;
}

export interface SaveSettingsRequest {
  settings: AppSettings;
  expectedRevision: number;
}

export interface FileRecord {
  id: string;
  name: string;
  path: string;
  directory: string;
  extension: string;
  size: number;
  file_type: FileType;
  purpose: Purpose;
  lifecycle: Lifecycle;
  context: string;
  risk_level: RiskLevel;
  hash: string | null;
  created_at: string;
  modified_at: string;
  scanned_at: string;
  last_seen_at: string;
  is_hidden: boolean;
  is_deleted: boolean;
  is_duplicate: boolean;
  suggested_action: SuggestedAction;
  suggested_target_path: string;
  suggested_name: string;
  confidence: number;
  classification_reason: string;
  classification_status: ClassificationStatus;
  matched_rules: string[];
  requires_confirmation: boolean;
  dispatch_zone?: DispatchZone;
  recommended_folder?: string;
  folder_reuse_candidate?: string;
  folder_rename_suggestion?: string;
  dispatch_reason?: string;
  next_action?: string;
  last_opened_at?: string | null;
  open_count?: number;
  indexed_at?: string;
  source_id?: string;
  is_stale?: boolean;
}

export interface GlobalVolume {
  id: string;
  platform: string;
  stableVolumeId: string;
  displayName: string;
  mountPath: string;
  filesystemType: string;
  driveKind: string;
  enabled: boolean;
  scopeAuthorizationState: "not_required" | "not_authorized" | "legacy_confirmation_required" | "authorized";
  provider: string;
  indexStatus: string;
  lastError: string | null;
  journalId: string | null;
  journalCursor: string | null;
  lastFullIndexAt: number | null;
  lastIncrementalSyncAt: number | null;
  entryCount: number;
  createdAt: number;
  updatedAt: number;
}

export interface GlobalSearchResult {
  id: string;
  volumeId: string;
  platformFileId: string;
  name: string;
  path: string;
  extension: string;
  isDirectory: boolean;
  size: number;
  createdAtFs: number | null;
  modifiedAtFs: number | null;
  fileAttributes: number;
  isHidden: boolean;
  isSystem: boolean;
  sourceProvider: string;
  managed: boolean;
  rank: number;
}

export interface GlobalSearchRequest {
  version: 2;
  requestId: string;
  query: string;
  limit: number;
  offset: number;
  cursor?: string | null;
}

export interface GlobalSearchSourceHealth {
  sourceId: string;
  enabled: boolean;
  provider: string;
  status: string;
  lastError: string | null;
  updatedAt: number;
}

export type GlobalSearchResultState = "pending" | "complete" | "partial" | "empty" | "failed" | "no_source";

export interface GlobalSearchResponse {
  version: 2;
  requestId: string;
  normalizedQuery: string;
  results: GlobalSearchResult[];
  indexStatus: GlobalIndexStatus;
  collectionComplete: boolean;
  resultState: GlobalSearchResultState;
  sourceRevision: string;
  sourceHealth: GlobalSearchSourceHealth[];
}

export interface GlobalIndexTechnicalDetail {
  journalId: string | null;
  journalCursor: string | null;
  provider: string;
  filesystemType: string;
}

export interface GlobalIndexSource {
  volume: GlobalVolume;
  canPause: boolean;
  canRebuild: boolean;
  technicalDetail: GlobalIndexTechnicalDetail | null;
}

export interface GlobalIndexStatus {
  platform: string;
  enabled: boolean;
  status: string;
  providerStatus?: string | null;
  processedEntries: number;
  collectionComplete: boolean;
  totalEntries: number;
  indexedVolumes: number;
  readyVolumes: number;
  pendingVolumes: number;
  lastSyncAt: number | null;
  lastError: string | null;
}

export interface ManagedScope {
  id: string;
  path: string;
  globalEntryId: string | null;
  enabled: boolean;
  allowLocalAi: boolean;
  allowCloudAi: boolean;
  createdAt: number;
  updatedAt: number;
}

export interface AiManagementStatus {
  enabledScopeCount: number;
  managedEntryCount: number;
  pendingJobCount: number;
  runningJobCount: number;
  cloudScopeCount: number;
  policySummary: string;
}

export interface AddManagedScopeRequest {
  path: string;
  globalEntryId?: string | null;
  enabled?: boolean;
  allowLocalAi?: boolean;
  allowCloudAi?: boolean;
}

export interface UpdateManagedScopePolicyRequest {
  id: string;
  enabled?: boolean;
  allowLocalAi?: boolean;
  allowCloudAi?: boolean;
}

export interface ScanRoot {
  id: string;
  path: string;
  platform: NodeJS.Platform | string;
  enabled: boolean;
  last_scanned_at: string | null;
  created_at: string;
  disk_total_size?: number | null;
  disk_free_size?: number | null;
  scanned_size?: number;
  indexed_file_count?: number;
  skipped_count?: number;
  summarized_count?: number;
}

export type RuleSource = "system" | "user" | "session" | "ai" | "learned" | "unknown";
export type RuleOperator = "AND" | "OR" | "UNKNOWN";

export type ConditionField =
  | "name"
  | "extension"
  | "file_type"
  | "path"
  | "directory"
  | "size"
  | "modified_at"
  | "is_duplicate"
  | "risk_level"
  | "unknown";

export type ConditionOperator =
  | "contains"
  | "equals"
  | "startsWith"
  | "endsWith"
  | "greaterThan"
  | "lessThan"
  | "olderThanDays"
  | "newerThanDays"
  | "is"
  | "unknown";

export interface RuleCondition {
  id: string;
  field: ConditionField;
  operator: ConditionOperator;
  value: string | number | boolean;
}

export interface RuleConditionGroup {
  id: string;
  operator: RuleOperator;
  conditions: RuleCondition[];
}

export interface RuleAction {
  purpose?: Purpose;
  lifecycle?: Lifecycle;
  context?: string;
  risk_level?: RiskLevel;
  suggested_action?: SuggestedAction;
  target_template?: string;
  rename_template?: string;
}

export interface Rule {
  id: string;
  name: string;
  source: RuleSource;
  enabled: boolean;
  priority: number;
  weight: number;
  root_operator: RuleOperator;
  groups: RuleConditionGroup[];
  action: RuleAction;
  created_at: string;
  updated_at: string;
  astVersion?: number;
  revision?: number;
  originProposalId?: string | null;
}

export interface RuleDraftV2 {
  name: string;
  priority: number;
  weight: number;
  rootOperator: "AND" | "OR";
  groups: Array<{
    operator: "AND" | "OR";
    conditions: Array<{
      field: Exclude<ConditionField, "unknown">;
      operator: Exclude<ConditionOperator, "unknown">;
      value: string | number | boolean;
    }>;
  }>;
  action: {
    purpose?: Purpose;
    lifecycle?: Lifecycle;
    context?: string;
    riskLevel?: RiskLevel;
    suggestedAction?: SuggestedAction;
    targetTemplate?: string;
    renameTemplate?: string;
  };
}

export interface RuleCatalogState {
  revision: number;
  updatedAt: number;
}

export interface RuleMutationResultV2 {
  rule: Rule;
  catalogRevision: number;
}

export interface RuleExecutionResultV2 {
  summary: RuleExecutionSummary;
  catalogRevision: number;
  classificationVersion: string;
}

export type RuleProposalStatus =
  | "draft"
  | "generating"
  | "ready"
  | "needs_clarification"
  | "invalid"
  | "failed"
  | "stale"
  | "applying"
  | "applied"
  | "cancelled";

export interface RuleProposalValidation {
  valid: boolean;
  permissionClass: "allow" | "ask" | "deny";
  requiresConfirmation: boolean;
  broadMatch: boolean;
  codes: string[];
  warnings: string[];
}

export interface CanonicalRuleAstV1 {
  astVersion: 1;
  name: string;
  priority: number;
  weight: number;
  rootOperator: "AND" | "OR";
  groups: RuleConditionGroup[];
  action: RuleAction;
}

export interface RuleProposal {
  id: string;
  status: RuleProposalStatus;
  intentKind: "create" | "update";
  targetRuleId: string | null;
  baseRuleRevision: number | null;
  prompt: string;
  promptFingerprint: string;
  providerKind: AIProviderKind | null;
  providerPreset: AIProviderPresetId | null;
  model: string | null;
  candidateOrigin?: "provider" | "manual" | string;
  astVersion: number;
  candidate: CanonicalRuleAstV1 | null;
  candidateFingerprint: string | null;
  summary: string | null;
  clarifications: string[];
  validation: RuleProposalValidation;
  appliedRuleId: string | null;
  revision: number;
  lastErrorCode: string | null;
  lastErrorDetail: string | null;
  createdAt: number;
  updatedAt: number;
  generatedAt: number | null;
  appliedAt: number | null;
}

export interface RuleProposalPage {
  proposals: RuleProposal[];
  nextCursor: string | null;
  hasMore: boolean;
}

export interface RuleImpactSampleRow {
  fileId: string;
  name: string;
  extension: string;
  size: number;
  modifiedAt: number;
  fileType: string;
  riskLevel: string;
  beforeAction: string;
  afterAction: string | null;
  beforePurpose?: string;
  afterPurpose?: string | null;
  beforeTargetPath?: string;
  afterTargetPath?: string | null;
  beforeReason?: string;
  afterReason?: string | null;
  beforeRequiresConfirmation?: boolean;
  afterRequiresConfirmation?: boolean | null;
  beforeWinnerRule?: string | null;
  beforeRunnerRule?: string | null;
  afterWinnerRule?: string | null;
  afterRunnerRule?: string | null;
}

export interface RuleConflictPreview {
  ruleId: string;
  name: string;
  kind: string;
}

export interface RuleProposalImpact {
  proposalId: string;
  proposalRevision: number;
  candidateFingerprint: string;
  catalogRevision: number;
  libraryRevision: number;
  scopeHealth: LibraryScopeHealth;
  permissionClass: "allow" | "ask" | "deny";
  impactState: "exact" | "deferred";
  matchedCount: number | null;
  impactToken: string | null;
  sampleRows: RuleImpactSampleRow[];
  sampleIsBounded: boolean;
  actionSummary: RuleAction;
  riskSummary: string[];
  requiresConfirmation: boolean;
  broadMatch: boolean;
  conflictAnalysisState: string;
  conflicts: RuleConflictPreview[];
  previewFingerprint: string;
}

export interface ApplyRuleProposalResult {
  proposal: RuleProposal;
  rule: Rule;
  catalogRevision: number;
}

export interface ContentScopePolicy {
  rootId: string;
  rootRevision: number;
  enabled: boolean;
  extractorFamilies: string[];
  maxBytes: number;
  maxChars: number;
  maxPages: number;
  maxRows: number;
  rawRetentionMode: string;
  rawRetentionChars: number;
  localAllowed: boolean;
  cloudAllowed: boolean;
  policyRevision: number;
  updatedAt: number;
}

export interface ContentPolicyRevisionRequest {
  rootId: string;
  rootRevision: number;
  policyRevision: number;
}

export interface ContentPreviewRequest {
  version: 1;
  requestId: string;
  scope: FileLibraryScopeV2;
  selectionFileIds: string[];
  mode: "local" | "understand" | "local_and_understand";
  expectedLibraryRevision: number;
  expectedPolicyRevisions: ContentPolicyRevisionRequest[];
  providerMode: "none" | "existing_interactive_provider";
}

export interface ContentSample {
  fileId: string;
  name: string;
  extension: string;
  size: number;
  modifiedAt: number;
  status: string;
  extractorFamily: string | null;
  reason: string | null;
}

export interface ContentPreview {
  version: number;
  requestId: string;
  scopeHealth: { scope: FileLibraryScopeV2; health: LibraryScopeHealth; rootIds: string[]; policyRevisions: ContentPolicyRevisionRequest[] };
  exactCount: number;
  deferredCount: number | null;
  exactState: string;
  candidateResolver: string;
  candidateFingerprint: string;
  perFileByteBudget: number;
  perFileCharBudget: number;
  totalByteBudget: number;
  totalCharBudget: number;
  byteBudget: number;
  charBudget: number;
  supportedCount: number;
  unsupportedCount: number;
  blockedCount: number;
  failedCount: number;
  supportedFormats: string[];
  unsupportedFormats: string[];
  blockedReasons: string[];
  localAllowed: boolean;
  cloudAllowed: boolean;
  rawRetentionDisclosure: string;
  sample: ContentSample[];
  libraryRevision: number;
  policyFingerprint: string;
  previewFingerprint: string;
  requiresConfirmation: boolean;
}

export interface ContentRun {
  id: string;
  scope: FileLibraryScopeV2;
  mode: string;
  providerMode: string;
  status: string;
  expectedLibraryRevision: number;
  candidateFingerprint: string;
  candidateResolver: string;
  byteBudget: number;
  charBudget: number;
  requestedCount: number;
  materializedCount: number;
  completedCount: number;
  blockedCount: number;
  skippedCount: number;
  failedCount: number;
  providerRevision: number;
  providerConfirmed: boolean;
  cancelRequested: boolean;
  revision: number;
  lastErrorCode: string | null;
  lastErrorDetail: string | null;
  createdAt: number;
  updatedAt: number;
  completedAt: number | null;
}

export interface ActiveContentRunForFile {
  run: ContentRun;
  items: ContentRunItem[];
}

export interface ContentRunItem {
  id: string;
  runId: string;
  fileId: string;
  ordinal: number;
  status: string;
  rootId: string | null;
  sourceIsDir: boolean;
  sourceSize: number;
  sourceMtime: number;
  sourceHash: string;
  extractorFamily: string | null;
  extractorVersion: string | null;
  artifactId: string | null;
  providerStatus: string;
  providerRevision: number;
  providerCompletedAt: number | null;
  errorCode: string | null;
  errorDetail: string | null;
  revision: number;
  updatedAt: number;
}

export interface ContentArtifact {
  id: string;
  fileId: string;
  scanRootId: string | null;
  sourceSize: number;
  sourceMtime: number;
  sourceIsDir: boolean;
  sourceHash: string;
  extractorFamily: string;
  extractorVersion: string;
  policyRevision: number;
  providerKind: string | null;
  providerModel: string | null;
  promptPolicyVersion: number | null;
  contentFingerprint: string;
  status: string;
  summary: string | null;
  keywords: string[];
  language: string | null;
  truncated: boolean;
  textRetained: boolean;
  provenance: unknown;
  revision: number;
  createdAt: number;
  updatedAt: number;
  lastRunId: string | null;
}

export interface ContentArtifactPage {
  artifacts: ContentArtifact[];
  nextCursor: string | null;
  hasMore: boolean;
  libraryRevision: number;
  contentRevision: number;
}

export interface FileQuery {
  search?: string;
  fileType?: FileType | "All";
  purpose?: Purpose | "All";
  lifecycle?: Lifecycle | "All";
  riskLevel?: RiskLevel | "All";
  sourceDirectory?: string;
  sortBy?: "name" | "size" | "modified_at" | "confidence";
  sortDirection?: "asc" | "desc";
  onlyActionable?: boolean;
  onlyNeedsConfirmation?: boolean;
  roots?: string[];
  limit?: number;
  offset?: number;
}

export interface FileQueryResult {
  files: FileRecord[];
  total: number;
  limit: number;
  offset: number;
}

export interface DashboardStats {
  totalFiles: number;
  totalSize: number;
  diskTotalSize: number;
  diskFreeSize: number;
  diskUsageRatio: number;
  duplicateFiles: number;
  largeFiles: number;
  sensitiveFiles: number;
  needsConfirmation: number;
  byType: Record<string, number>;
  byLifecycle: Record<string, number>;
  lastScannedAt: string | null;
}

export interface OperationPreview {
  id: string;
  fileId: string;
  file_id?: string;
  operation_type: OperationType;
  source_path: string;
  target_path: string;
  old_name: string;
  new_name: string;
  status: "pending" | "success" | "failed" | "skipped";
  risk_level: RiskLevel;
  confidence: number;
  requires_confirmation: boolean;
  suggested_action?: SuggestedAction;
  is_duplicate?: boolean;
  reason: string;
  selected_by_default?: boolean;
  is_executable?: boolean;
  blocking_reason?: string;
  editable_new_name?: boolean;
  batch_id?: string;
  target_parent_exists?: boolean;
  will_create_parent?: boolean;
  strategy?: string;
  conflict_policy?: string;
  will_copy?: boolean;
  will_move?: boolean;
  will_download?: boolean;
  materialization_requirement?: "none" | "metadata_only" | "explicit_download_required" | "required" | "provider_managed" | "unknown" | string;
  materializationRequirement?: "none" | "metadata_only" | "explicit_download_required" | "required" | "provider_managed" | "unknown" | string;
  operationFingerprint?: string;
  crossVolumeCopyRequired?: boolean;
  metadataDegradationPossible?: boolean;
  sourceRetirementCapability?: string;
  sourceRetirementEligible?: boolean;
  sourceRetirementProbeRequired?: boolean;
  providerCoordination?: boolean;
  sourceIdentityFingerprint?: string;
  providerIdentityFingerprint?: string;
  will_replace?: boolean;
  will_trash?: boolean;
}

export interface OperationPreviewResult {
  previews: OperationPreview[];
  total: number;
  limit: number;
  offset: number;
  truncated: boolean;
  hasMore: boolean;
}

export interface StorageCandidate {
  id: string;
  path: string;
  name: string;
  size: number;
  tier: CleanupTier;
  category: string;
  reason: string;
  suggested_action: CleanupActionKind;
  risk_note: string | null;
  trash_allowed: boolean;
  selected_by_default: boolean;
}

export interface CleanupFindingSelection {
  findingId: string;
  expectedRevision: number;
  reviewConfirmation?: {
    decisionRevision: number;
  };
}

export interface StorageAnalysis {
  total_size: number;
  reclaimable_estimate: number;
  review_estimate: number;
  candidates: StorageCandidate[];
  denied_paths: string[];
  warnings?: string[];
  candidate_total?: number;
  candidate_offset?: number;
  candidate_limit?: number;
  has_more?: boolean;
}

export interface StorageCleanupProgress {
  jobId: string;
  scannedEntries: number;
  currentPath: string | null;
  totalSize: number;
}

export interface CleanupRestoreProgressPayload {
  jobId: string;
  processed: number;
  total: number;
  currentItemId: string | null;
  currentPath: string | null;
  restored: number;
  conflicts: number;
  missing: number;
  failed: number;
  canceled: number;
  cancelRequested: boolean;
}

export interface StorageCleanupScanStatus {
  jobId: string;
  status: "queued" | "running" | "cancelling" | "completed" | "completed_with_warnings" | "failed" | "interrupted" | "cancelled";
  progress: StorageCleanupProgress;
  analysis: StorageAnalysis | null;
  error: string | null;
  startedAt: string;
  completedAt: string | null;
}

export interface StorageCleanupCompleted {
  jobId: string;
  analysis: StorageAnalysis;
}

export interface StorageCleanupJobMessage {
  jobId: string;
  message: string;
}

export interface CleanupPreviewItem {
  id: string;
  candidate_id: string;
  path: string;
  name: string;
  size: number;
  tier: CleanupTier;
  category: string;
  reason: string;
  operation_type: "move_to_trash_preview";
  target_path: string;
  status: "pending" | "success" | "failed" | "skipped";
  requires_confirmation: boolean;
  is_executable: boolean;
  blocking_reason: string | null;
}

export interface CleanupExecutionLog {
  path: string;
  name: string;
  size: number;
  status: "success" | "skipped" | "failed" | "manual_review";
  message: string;
  itemId?: string | null;
  trashPath?: string | null;
}

export interface CleanupExecutionResult {
  moved: number;
  skipped: number;
  failed: number;
  logs: CleanupExecutionLog[];
}

export interface CleanupTrashItem {
  id: string;
  batchId: string;
  originalPath: string;
  trashPath: string;
  name: string;
  size: number;
  movedAt: string;
  restoredAt: string | null;
  status: "pending" | "moved" | "restored" | "missing" | "failed" | "manual_review" | "canceled";
  message: string | null;
  sourceClaimPath?: string | null;
}

export interface CleanupTrashBatch {
  id: string;
  createdAt: string;
  root: string | null;
  totalItems: number;
  totalSize: number;
  status: "pending" | "success" | "partial_failed";
  items: CleanupTrashItem[];
}

export interface CleanupRestorePreview {
  batchId: string;
  items: CleanupRestorePreviewItem[];
}

export interface CleanupRestorePreviewItem extends CleanupTrashItem {
  canRestore: boolean;
  blockingReason: string | null;
}

export interface CleanupRestoreLog {
  itemId: string;
  originalPath: string;
  trashPath: string;
  status: "restored" | "conflict" | "missing" | "failed" | "manual_review" | "canceled";
  message: string;
}

export interface CleanupRestoreResult {
  restored: number;
  conflicts: number;
  missing: number;
  failed: number;
  canceled: number;
  logs: CleanupRestoreLog[];
}

export interface OperationLog {
  id: string;
  batch_id: string;
  operation_type: string;
  source_path: string;
  target_path: string;
  old_name: string;
  new_name: string;
  status: "pending" | "success" | "failed" | "skipped" | "manual_review";
  error_message: string | null;
  created_at: string;
  can_undo: boolean;
  path_before: string;
  path_after: string;
  name_before: string;
  name_after: string;
  can_restore: boolean;
  restored_at: string | null;
  restore_status: RestoreStatus;
  restore_error: string | null;
  source_size?: number | null;
  source_modified_ns?: string | null;
  source_platform_file_id?: string | null;
  source_quick_hash?: string | null;
  source_full_hash?: string | null;
  target_platform_file_id?: string | null;
  target_full_hash?: string | null;
  source_claim_path?: string | null;
  operation_phase?: string;
  claim_created_at?: string | null;
  claim_platform_file_id?: string | null;
  claim_full_hash?: string | null;
  restore_claim_path?: string | null;
  restore_phase?: RestorePhase;
  restore_claim_created_at?: string | null;
  restore_claim_platform_file_id?: string | null;
  restore_claim_full_hash?: string | null;
}

export interface ExecuteOperationRequest {
  operations: Array<{
    id: string;
    fileId: string;
    operationFingerprint: string;
    expectedRevision: string;
    newName?: string;
    destinationDirectory?: string;
  }>;
}

export interface ExecuteOperationResult {
  logs: OperationLog[];
  batch_id: string;
}

export interface RestoreMovesResult {
  logs: OperationLog[];
  restored: number;
  failed: number;
}

export type RecoveryAction = "keep_both" | "move" | "delete" | "retry_cleanup";

export interface RecoveryActionResult {
  original_log: OperationLog;
  action_log: OperationLog;
  target_path: string | null;
}

export interface SearchSource {
  id: string;
  label: string;
  path: string;
  type: SearchSourceType;
  enabled: boolean;
  is_stale: boolean;
  indexed_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface SearchIndexState {
  total_files: number;
  indexed_files: number;
  last_indexed_at: string | null;
  stale_sources: number;
}

export interface SearchQuery {
  query: string;
  limit?: number;
  sourceIds?: string[];
}

export interface SearchResult {
  file: FileRecord;
  score: number;
  matched_text: string;
}

export interface RestoreBatch {
  batch_id: string;
  created_at: string;
  total: number;
  success: number;
  failed: number;
  skipped: number;
  restorable: number;
  restored: number;
  expires_at: string;
}

export interface RestorePreviewItem {
  log_id: string;
  batch_id: string;
  operation_type: string;
  current_path: string;
  restore_path: string;
  old_name: string;
  new_name: string;
  can_restore: boolean;
  blocking_reason: string | null;
}

export interface RestorePreview {
  batch_id: string;
  items: RestorePreviewItem[];
}

export interface RestoreBatchResult {
  batch_id: string;
  restored: number;
  failed: number;
  skipped: number;
  items: RestorePreviewItem[];
}

export interface ScanResult {
  roots: ScanRoot[];
  files: FileRecord[];
  skipped: Array<{ path: string; reason: string }>;
  scannedAt: string;
  canceled?: boolean;
}

export interface FolderScanResult extends ScanResult {
  canceled: boolean;
  selectedPaths: string[];
}

export type ScanPhase = "queued" | "scanning" | "indexing" | "done" | "canceled" | "error";

export interface ScanProgress {
  scanId: string;
  phase: ScanPhase;
  currentPath: string | null;
  scannedFiles: number;
  indexedFiles: number;
  skipped: number;
  summarized: number;
  rootsTotal: number;
  rootsDone: number;
  message?: string;
}

export interface AppSnapshot {
  stats: DashboardStats;
  files: FileRecord[];
  rules: Rule[];
  operations: OperationLog[];
  scanRoots: ScanRoot[];
  searchSources: SearchSource[];
  searchIndex: SearchIndexState;
}
