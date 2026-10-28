import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (file: string) => readFileSync(resolve(file), "utf8");

describe("W2-11 integrated experience QA contract", () => {
  it("keeps the deterministic 100k fixture and real-gate evidence seam explicit", () => {
    const gate = read("scripts/runW2-11BrowserGate.mjs");
    const libraryMock = read("src/api/browserMockApi.ts");
    const browseMock = read("src/api/fileWorkspaceMockApi.ts");

    expect(gate).toContain('const FIXTURE_QUERY = "w2-11-browser-fixture=integrated"');
    expect(gate).toContain("const LIBRARY_TOTAL = 100_000");
    expect(gate).toContain("const BROWSE_SCAN_BUDGET = 1_024");
    expect(gate).toContain("const BROWSE_LATE_SENTINEL_INDEX = 99_000");
    expect(libraryMock).toContain("const W211_LIBRARY_TOTAL = 100_000");
    expect(browseMock).toContain("const W211_BROWSE_TOTAL = 100_000");
    expect(browseMock).toContain("const W211_SCAN_BUDGET = 1_024");
    expect(browseMock).toContain("const W211_LATE_SENTINEL_INDEX = 99_000");
    expect(gate).toContain("const PLATEAU_CYCLE_COUNT = 8");
    expect(gate).toContain("const PLATEAU_IGNORED_CYCLES = 2");
    expect(gate).toContain("const PLATEAU_MAX_CONSECUTIVE_POSITIVE_LISTENER_DELTAS = 2");
    expect(gate).toContain("const PLATEAU_MAX_LATER_LISTENER_SPREAD = 48");
    expect(gate).toContain("async function warmUpInteractionSurface");
    expect(gate).toContain("async function runResourcePlateauCycle");
    expect(gate).toContain("listenerNetSpread");
    expect(gate).toContain("maxConsecutivePositiveDeltas");
    expect(gate).toContain("durableListenerNet");
    expect(gate).toContain("window/document/MediaQueryList only");
    expect(gate).toContain("plateau: result.resources.plateau");
    expect(gate).toContain("[w2-11-real] PASS");
  });

  it("captures bounded and redacted browser resource evidence without weakening the error gate", () => {
    const gate = read("scripts/runW2-11BrowserGate.mjs");

    expect(gate).toContain("const MAX_NETWORK_DIAGNOSTIC_ITEMS = 64");
    expect(gate).toContain('page.on("requestfailed"');
    expect(gate).toContain('page.on("response"');
    expect(gate).toContain("message.location()");
    expect(gate).toContain("request.failure()?.errorText");
    expect(gate).toContain("responseStatusCounts");
    expect(gate).toContain("requestFailuresDropped");
    expect(gate).toContain("url.username = \"\"");
    expect(gate).toContain("url.password = \"\"");
    expect(gate).toContain("url.search = \"\"");
    expect(gate).toContain("<local-path>");
    expect(gate).toContain("localFilePath = pathname.match");
    expect(gate).toContain("    networkDiagnostics,");
    expect(gate).toContain("assert(consoleErrors.length === 0 && pageErrors.length === 0,");
  });

  it("keeps sparse Browse search yielded, batched, and generation-bound", () => {
    const owner = read("src/views/fileLibrary/browse/browseSourceOwner.ts");

    expect(owner).toContain("const QUERY_SCAN_PAGE_BATCH = 8");
    expect(owner).toContain("const loadNextQueryPage = useCallback");
    expect(owner).toContain("generationRef.current !== generation");
    expect(owner).toContain("window.setTimeout");
    expect(owner).toContain("window.clearTimeout");
    expect(owner).toContain("void loadNextQueryPage()");
  });

  it("does not rerun virtualizer demand effects from a rebuilt projection object", () => {
    const list = read("src/views/fileLibrary/list/SharedFileList.tsx");
    const grid = read("src/views/fileLibrary/list/SharedFileGrid.tsx");

    expect(list).toContain("interaction.actions.loadMore");
    expect(grid).toContain("interaction.actions.loadMore");
    expect(list).not.toContain("}, [interaction, lastVisibleIndex, rowVirtualizer]");
    expect(grid).not.toContain("}, [columns, interaction, lastVisibleRow, rowVirtualizer]");
  });
});
