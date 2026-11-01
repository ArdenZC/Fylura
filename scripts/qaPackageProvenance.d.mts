export interface QaInstallerArtifact {
  filename: string;
  sizeBytes: number;
  sha256: string;
}

export interface QaPackageArchitecture {
  runnerOs: string;
  runnerArchitecture: string;
  nodeArchitecture: string;
  rustTarget: string;
  application: unknown;
}

export interface QaPackageProvenance {
  schemaVersion: 1;
  classification: "QA_ONLY_UNQUALIFIED";
  qualification: {
    releaseQualified: false;
    installedGuiAcceptance: "not claimed";
    packageSigning: string;
  };
  source: {
    repository: string;
    sha: string;
    tree: string;
    ref: string;
  };
  workflow: {
    file: string;
    event: "workflow_dispatch";
    runId: string;
    runAttempt: string;
  };
  build: {
    runner: {
      platform: string;
      architecture: string;
    };
    cargoFeatures: string[];
    filenameCandidate: {
      compiled: boolean;
      runtimeOptInVariable: "ZC_GLOBAL_SEARCH_FILENAME_RELEVANCE_CANDIDATE";
      defaultState: "off";
      enableValue: "1";
    };
    application: QaPackageArchitecture;
  };
  package: {
    type: string;
    version: string;
    checksumManifest: string;
    artifacts: QaInstallerArtifact[];
  };
}

export function verifyInstallerChecksums(files: string[], contents: string): QaInstallerArtifact[];

export function buildQaPackageProvenance(input: {
  sourceSha: string;
  sourceTree: string;
  repository: string;
  ref: string;
  runnerOs: string;
  runnerArchitecture: string;
  runId: string | number;
  runAttempt: string | number;
  version: string;
  artifactType: string;
  architecture: QaPackageArchitecture;
  cargoFeatures: string[];
  artifacts: QaInstallerArtifact[];
  checksumManifest: string;
}): QaPackageProvenance;

