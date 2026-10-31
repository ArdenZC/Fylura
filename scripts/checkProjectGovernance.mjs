import { existsSync, readFileSync, readdirSync } from "node:fs";
import { resolve } from "node:path";

const root = process.cwd();
const failures = [];
const fileCache = new Map();

function absolute(relativePath) {
  return resolve(root, relativePath);
}

function readRequired(relativePath) {
  if (fileCache.has(relativePath)) return fileCache.get(relativePath);

  const file = absolute(relativePath);
  if (!existsSync(file)) {
    failures.push(`${relativePath}: required governance file does not exist`);
    fileCache.set(relativePath, "");
    return "";
  }

  try {
    const contents = readFileSync(file, "utf8");
    if (contents.trim().length === 0) {
      failures.push(`${relativePath}: required governance file is empty`);
    }
    fileCache.set(relativePath, contents);
    return contents;
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    failures.push(`${relativePath}: required governance file could not be read (${reason})`);
    fileCache.set(relativePath, "");
    return "";
  }
}

function sectionAfterHeading(markdown, heading) {
  const headingIndex = markdown.indexOf(heading);
  if (headingIndex < 0) return "";
  const contentStart = headingIndex + heading.length;
  const nextHeading = markdown.indexOf("\n## ", contentStart);
  return markdown.slice(contentStart, nextHeading < 0 ? markdown.length : nextHeading);
}

function normalizeTitle(title) {
  return title.replace(/[`*_]/gu, "").trim();
}

function collapseWhitespace(value) {
  return value.replace(/\s+/gu, " ").trim();
}

function hasExplicitNonTargetFact(markdown, tokenPattern) {
  const normalized = collapseWhitespace(markdown);
  const nonTargetPatterns = [
    new RegExp(`\\b${tokenPattern}\\b[^.!?]{0,180}\\b(?:is|are)\\s+(?:not|unsupported)\\s+(?:a\\s+)?(?:product\\s+targets?|supported\\s+product\\s+platforms?)\\b`, "iu"),
    new RegExp(`\\b${tokenPattern}\\b[^.!?]{0,180}\\boutside\\s+product\\s+(?:support|targets?)\\b`, "iu")
  ];
  return nonTargetPatterns.some((pattern) => pattern.test(normalized));
}

function activeInitiativeMode(value) {
  if (!/\bactive\b/iu.test(value)) return null;
  if (/\bspecification\s+only\b/iu.test(value)) return "specification";
  if (/\bimplementation\b/iu.test(value)) return "implementation";
  return null;
}

function isActiveInitiativeStatus(value) {
  return /\bactive\b/iu.test(value ?? "");
}

function isBetweenInitiatives(title, status) {
  return normalizeTitle(title ?? "").toLowerCase() === "no active initiative"
    && /\bbetween\s+initiatives\b/iu.test(status ?? "")
    && /\bno\s+active\b/iu.test(status ?? "");
}

function readInitiativeRecords() {
  const directory = "docs/project/initiatives";
  const absoluteDirectory = absolute(directory);
  if (!existsSync(absoluteDirectory)) {
    failures.push(`${directory}: initiative directory does not exist`);
    return [];
  }

  return readdirSync(absoluteDirectory, { withFileTypes: true })
    .filter((entry) => entry.isFile() && entry.name.endsWith(".md") && entry.name !== "TEMPLATE.md")
    .map((entry) => {
      const relativePath = `${directory}/${entry.name}`;
      const contents = readRequired(relativePath);
      const titleMatch = contents.match(/^#\s+(.+?)\s*$/mu);
      const statusMatch = contents.match(/^Status:\s*(.+)$/mu);
      if (!titleMatch) failures.push(`${relativePath}: initiative record main title is missing`);
      if (!statusMatch) failures.push(`${relativePath}: initiative record status is missing`);
      const status = statusMatch?.[1] ?? "";
      if (isActiveInitiativeStatus(status) && !activeInitiativeMode(status)) {
        failures.push(`${relativePath}: active initiative must declare 'specification only' or 'implementation'`);
      }
      return {
        relativePath,
        title: titleMatch?.[1] ?? "",
        status,
        active: isActiveInitiativeStatus(status)
      };
    });
}

const requiredGovernanceFiles = [
  "docs/project/README.md",
  "docs/project/STATUS.md",
  "docs/project/ARCHITECTURE_MAP.md",
  "docs/project/ROADMAP.md",
  "docs/project/TECH_DEBT.md",
  "docs/project/RISK_REGISTER.md",
  "docs/project/DEVELOPMENT_WORKFLOW.md",
  "docs/security/SUPPORTED_PLATFORMS.md",
  "docs/project/research/file-library-preview/OPEN_SOURCE_SYNTHESIS.md"
];

for (const relativePath of requiredGovernanceFiles) readRequired(relativePath);

const statusPath = "docs/project/STATUS.md";
const status = readRequired(statusPath);
const roadmap = readRequired("docs/project/ROADMAP.md");
const agents = readRequired("AGENTS.md");
const supportedPlatforms = readRequired("docs/security/SUPPORTED_PLATFORMS.md");
const w0Path = "docs/project/initiatives/W0-file-library-preview.md";
const w0 = readRequired(w0Path);
const initiativeRecords = readInitiativeRecords();
const activeInitiativeRecords = initiativeRecords.filter((record) => record.active);

if (existsSync(absolute("CLAUDE.md"))) {
  failures.push("CLAUDE.md: retired root instruction file must not exist");
}

const statusCurrentHeading = "## Current initiative";
const statusCurrentCount = (status.match(/^## Current initiative\s*$/gmu) ?? []).length;
if (statusCurrentCount !== 1) {
  failures.push(`STATUS.md: expected exactly one '${statusCurrentHeading}' section`);
}

const statusCurrent = sectionAfterHeading(status, statusCurrentHeading);
const statusTitleMatch = statusCurrent.match(/^\*\*(.+?)\*\*\s*$/mu);
const statusLineMatch = statusCurrent.match(/^Status:\s*(.+)$/mu);

if (!statusTitleMatch) failures.push("STATUS.md: current initiative name is missing");
if (!statusLineMatch) failures.push("STATUS.md: current initiative status is missing");

const roadmapCurrentCount = (roadmap.match(/^## Current\s*$/gmu) ?? []).length;
if (roadmapCurrentCount !== 1) {
  failures.push("ROADMAP.md: expected exactly one '## Current' section");
}

const roadmapCurrent = sectionAfterHeading(roadmap, "## Current");
const roadmapTitleMatch = roadmapCurrent.match(/^###\s+(.+?)\s*$/mu);
const roadmapStatusMatch = roadmapCurrent.match(/^Status:\s*(.+)$/mu);
const roadmapCurrentEntries = roadmapCurrent.match(/^###\s+.+$/gmu) ?? [];

if (roadmapCurrentEntries.length !== 1) {
  failures.push("ROADMAP.md: current section must contain exactly one current-state entry");
}
if (!roadmapTitleMatch) failures.push("ROADMAP.md: current initiative name is missing");
if (!roadmapStatusMatch) failures.push("ROADMAP.md: current initiative status is missing");

if (statusTitleMatch && roadmapTitleMatch) {
  const statusTitle = normalizeTitle(statusTitleMatch[1]);
  const roadmapTitle = normalizeTitle(roadmapTitleMatch[1]);
  if (statusTitle !== roadmapTitle) {
    failures.push(`current initiative mismatch: STATUS.md='${statusTitle}' ROADMAP.md='${roadmapTitle}'`);
  }
}

const statusBetween = isBetweenInitiatives(statusTitleMatch?.[1], statusLineMatch?.[1]);
const roadmapBetween = isBetweenInitiatives(roadmapTitleMatch?.[1], roadmapStatusMatch?.[1]);

if (statusBetween !== roadmapBetween) {
  failures.push("between-initiatives state mismatch between STATUS.md and ROADMAP.md");
}

if (statusBetween && roadmapBetween) {
  if (/\]\((initiatives\/[^)]+\.md)\)/u.test(statusCurrent)) {
    failures.push("STATUS.md: between-initiatives state must not point to an active initiative record");
  }
  if (/\]\((initiatives\/[^)]+\.md)\)/u.test(roadmapCurrent)) {
    failures.push("ROADMAP.md: between-initiatives state must not point to an active initiative record");
  }
  if (activeInitiativeRecords.length > 0) {
    failures.push(
      `between-initiatives state requires zero active initiative records; found ${activeInitiativeRecords.map((record) => record.relativePath).join(", ")}`
    );
  }
} else {
  const initiativeLinkMatch = statusCurrent.match(/\]\((initiatives\/[^)]+\.md)\)/u);
  let initiativeRecord = "";
  let initiativeTitleMatch;
  let initiativeStatusMatch;
  let linkedPath = "";

  if (!initiativeLinkMatch) {
    failures.push("STATUS.md: current initiative must link to its initiative record");
  } else {
    linkedPath = `docs/project/${initiativeLinkMatch[1]}`;
    initiativeRecord = readRequired(linkedPath);
    if (initiativeRecord) {
      initiativeTitleMatch = initiativeRecord.match(/^#\s+(.+?)\s*$/mu);
      initiativeStatusMatch = initiativeRecord.match(/^Status:\s*(.+)$/mu);
      if (!initiativeTitleMatch) failures.push(`${linkedPath}: initiative record main title is missing`);
      if (!initiativeStatusMatch) failures.push(`${linkedPath}: initiative record status is missing`);
    }
  }

  if (activeInitiativeRecords.length !== 1) {
    failures.push(`active project state requires exactly one active initiative record; found ${activeInitiativeRecords.length}`);
  } else if (linkedPath && activeInitiativeRecords[0].relativePath !== linkedPath) {
    failures.push(`STATUS.md active initiative link '${linkedPath}' does not match the sole active initiative record '${activeInitiativeRecords[0].relativePath}'`);
  }

  if (statusTitleMatch && initiativeTitleMatch) {
    const statusTitle = normalizeTitle(statusTitleMatch[1]);
    const initiativeTitle = normalizeTitle(initiativeTitleMatch[1]);
    if (statusTitle !== initiativeTitle) {
      failures.push(`current initiative mismatch: STATUS.md='${statusTitle}' initiative='${initiativeTitle}'`);
    }
  }

  const initiativeModes = [
    ["STATUS.md", statusLineMatch],
    ["ROADMAP.md", roadmapStatusMatch],
    ["current initiative record", initiativeStatusMatch]
  ].map(([source, lineMatch]) => {
    if (!lineMatch) return [source, null];
    const mode = activeInitiativeMode(lineMatch[1]);
    if (!mode) {
      failures.push(`${source}: current initiative must be active and declare 'specification only' or 'implementation'`);
    }
    return [source, mode];
  });

  const declaredModes = initiativeModes.map(([, mode]) => mode).filter(Boolean);
  if (new Set(declaredModes).size > 1) {
    failures.push(`current initiative status mode mismatch: ${initiativeModes.map(([source, mode]) => `${source}=${mode ?? "invalid"}`).join(" ")}`);
  }

  if (statusTitleMatch) {
    const statusTitle = normalizeTitle(statusTitleMatch[1]);
    const currentMode = activeInitiativeMode(statusLineMatch?.[1] ?? "");
    if (/\bW0\s+Specification\b/iu.test(statusTitle) && currentMode !== "specification") {
      failures.push("W0 Specification must remain active specification only while it is current");
    }
  }
}

const hardcodedAgentPatterns = [
  /^\s*Current(?:\s+project)?\s+(?:phase|stage|task|initiative)\s*[:=]/iu,
  /^\s*Active(?:\s+project)?\s+(?:phase|stage|task|initiative)\s*[:=]/iu,
  /^\s*Current(?:\s+implementation\/repository)?\s+baseline\s*[:=]\s*`?[0-9a-f]{7,}/iu,
  /^\s*Current\s+task\s*[:=]/iu
];

agents.split(/\r?\n/u).forEach((line, index) => {
  if (hardcodedAgentPatterns.some((pattern) => pattern.test(line))) {
    failures.push(`AGENTS.md:${index + 1}: changing project stage/baseline/task must live in STATUS.md`);
  }
});

if (/\bW-1 Open Source Research\s*[—-]\s*completed\b/iu.test(w0)
  && !readRequired("docs/project/research/file-library-preview/OPEN_SOURCE_SYNTHESIS.md")) {
  failures.push("W0 initiative declares W-1 complete but OPEN_SOURCE_SYNTHESIS.md is missing or empty");
}

const supportedPlatformFacts = collapseWhitespace(supportedPlatforms);
if (supportedPlatforms) {
  if (!/Fylura\s+supports:\s*-\s*Windows\b/iu.test(supportedPlatformFacts)) {
    failures.push("SUPPORTED_PLATFORMS.md: Windows product target is missing");
  }
  if (!/macOS\s+13\s+or\s+later\s+on\s+Apple Silicon/iu.test(supportedPlatformFacts)) {
    failures.push("SUPPORTED_PLATFORMS.md: macOS 13 or later Apple Silicon target is missing");
  }
  if (!/\bApple Silicon\b/iu.test(supportedPlatformFacts)) {
    failures.push("SUPPORTED_PLATFORMS.md: Apple Silicon architecture fact is missing");
  }
  if (!/\baarch64-apple-darwin\b/iu.test(supportedPlatformFacts)) {
    failures.push("SUPPORTED_PLATFORMS.md: aarch64-apple-darwin target fact is missing");
  }
  for (const [label, tokenPattern] of [
    ["Intel Mac", "Intel Macs?"],
    ["Universal binary", "Universal binaries?"],
    ["Rosetta", "Rosetta"],
    ["Linux", "Linux"]
  ]) {
    if (!hasExplicitNonTargetFact(supportedPlatforms, tokenPattern)) {
      failures.push(`SUPPORTED_PLATFORMS.md: ${label} must be explicitly marked as not a product target`);
    }
  }
}

if (status) {
  for (const [label, tokenPattern] of [
    ["Intel Mac", "Intel Macs?"],
    ["Universal binary", "Universal binaries?"],
    ["Rosetta", "Rosetta"],
    ["Linux", "Linux"]
  ]) {
    if (!hasExplicitNonTargetFact(status, tokenPattern)) {
      failures.push(`STATUS.md: ${label} support must not be claimed; mark it as not a product target`);
    }
  }
}

if (failures.length > 0) {
  console.error("Project governance validation failed:\n");
  failures.forEach((failure) => console.error(`- ${failure}`));
  process.exit(1);
}

console.log("Project governance validation passed.");
