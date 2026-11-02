export const W201_VIEWPORTS = Object.freeze({
  wide: Object.freeze({ width: 1600, height: 900 }),
  medium: Object.freeze({ width: 1280, height: 720 }),
  compact: Object.freeze({ width: 980, height: 680 })
});

/**
 * Collects the W2-01 layout contract from the real rendered page.
 *
 * Keep this function self-contained: it is passed directly to the browser
 * evaluate API, so it must not close over module-level helpers.
 */
export function collectW201BrowserMeasurement(sourceHead = null, requestedViewport = null) {
  if (Array.isArray(sourceHead) && requestedViewport === null) {
    [sourceHead, requestedViewport] = sourceHead;
  }
  // The workbench remains mounted but is hidden when the user visits another
  // view. Only visible scene elements participate in layout measurements.
  const select = (selector, root = document) => {
    const element = root.querySelector(selector);
    return element?.closest("[hidden]") ? null : element;
  };
  const rect = (element) => {
    if (!element) return null;
    const box = element.getBoundingClientRect();
    return {
      top: box.top,
      bottom: box.bottom,
      height: box.height,
      left: box.left,
      right: box.right,
      width: box.width
    };
  };
  const scroll = (element) => {
    if (!element) return null;
    const styles = getComputedStyle(element);
    return {
      clientHeight: element.clientHeight,
      scrollHeight: element.scrollHeight,
      scrollTop: element.scrollTop,
      clientWidth: element.clientWidth,
      scrollWidth: element.scrollWidth,
      overflowY: styles.overflowY,
      overflowX: styles.overflowX
    };
  };
  const metric = (element) => element ? { bounds: rect(element), scroll: scroll(element) } : null;
  const findAncestor = (element, predicate) => {
    let current = element?.parentElement ?? null;
    while (current) {
      if (predicate(current)) return current;
      current = current.parentElement;
    }
    return null;
  };
  const root = select("#root");
  const workspace = select(".file-library-workspace");
  const titlebar = select("[data-shell-titlebar]") ?? select("header");
  const viewStage = findAncestor(workspace, (element) =>
    element.classList.contains("flex-1") && element.classList.contains("overflow-hidden")
  );
  const workspaceBody = select(".file-library-workspace-body");
  const contentSlot = select(".file-library-content-slot");
  const adapter = select(".file-library-library-adapter");
  const vaultRoot = select('[data-vault-presentation="embedded"]');
  const embeddedChrome = select('[data-vault-embedded-chrome="true"]');
  const resultRegion = select('[data-inspector-layout="true"]');
  const resultMain = select('[data-inspector-layout="true"] > div:first-child');
  const resultSection = select('[data-inspector-layout="true"] section');
  const listbox = select('[role="listbox"][data-file-library-scroll-owner="tanstack-virtualizer"]');
  const rows = [...document.querySelectorAll("[data-virtual-row-index]")]
    .map((element) => Number(element.getAttribute("data-virtual-row-index")))
    .filter(Number.isFinite)
    .sort((left, right) => left - right);
  const logicalCount = listbox ? Number(listbox.getAttribute("data-file-library-logical-count")) : null;
  const hasMoreAttribute = listbox?.getAttribute("data-file-library-has-more") ?? null;
  const bodyText = document.body?.innerText ?? "";
  const allResultsLoaded = /All results loaded|所有结果已加载/i.test(bodyText);

  const ancestorOverflow = [];
  let current = listbox?.parentElement ?? null;
  while (current && current !== root) {
    const styles = getComputedStyle(current);
    if (styles.overflowY === "hidden" || styles.overflowY === "clip") {
      ancestorOverflow.push({
        selector: current.getAttribute("data-workspace-slot")
          ? `[data-workspace-slot="${current.getAttribute("data-workspace-slot")}"]`
          : current.className || current.tagName.toLowerCase(),
        bounds: rect(current),
        overflowY: styles.overflowY
      });
    }
    current = current.parentElement;
  }

  return {
    sourceHead,
    capturedAt: new Date().toISOString(),
    viewportContract: {
      requested: requestedViewport,
      innerWidth: window.innerWidth,
      innerHeight: window.innerHeight,
      documentClientWidth: document.documentElement.clientWidth,
      documentClientHeight: document.documentElement.clientHeight,
      root: rect(root),
      matchesRequested: Boolean(
        requestedViewport
        && window.innerWidth === requestedViewport.width
        && window.innerHeight === requestedViewport.height
        && document.documentElement.clientWidth === requestedViewport.width
        && document.documentElement.clientHeight === requestedViewport.height
        && rect(root)?.width === requestedViewport.width
        && rect(root)?.height === requestedViewport.height
      )
    },
    bounds: {
      root: rect(root),
      titlebar: rect(titlebar),
      viewStage: rect(viewStage),
      workspace: rect(workspace),
      workspaceBody: rect(workspaceBody),
      contentSlot: rect(contentSlot),
      legacyLibraryAdapter: rect(adapter),
      vaultRoot: rect(vaultRoot),
      embeddedChrome: rect(embeddedChrome),
      resultRegion: rect(resultRegion),
      resultMain: rect(resultMain),
      resultSection: rect(resultSection),
      fileLibraryList: rect(listbox)
    },
    scrollOwnership: {
      document: scroll(document.documentElement),
      body: scroll(document.body),
      viewStage: scroll(viewStage),
      workspace: scroll(workspace),
      workspaceBody: scroll(workspaceBody),
      contentSlot: scroll(contentSlot),
      legacyLibraryAdapter: scroll(adapter),
      vaultRoot: scroll(vaultRoot),
      embeddedChrome: scroll(embeddedChrome),
      resultRegion: scroll(resultRegion),
      resultMain: scroll(resultMain),
      resultSection: scroll(resultSection),
      fileLibraryList: scroll(listbox),
      fileLibraryListSelector: listbox?.getAttribute("data-file-library-scroll-owner") ?? null,
      ancestorOverflow
    },
    virtualization: {
      logicalCount: Number.isFinite(logicalCount) ? logicalCount : null,
      hasMore: hasMoreAttribute === null ? null : hasMoreAttribute === "true",
      allResultsLoaded,
      mountedRowCount: rows.length,
      firstMountedRowIndex: rows[0] ?? null,
      lastMountedRowIndex: rows.at(-1) ?? null,
      mountedRowIndices: rows
    },
    page: {
      documentClientHeight: document.documentElement.clientHeight,
      documentScrollHeight: document.documentElement.scrollHeight,
      bodyClientHeight: document.body?.clientHeight ?? null,
      bodyScrollHeight: document.body?.scrollHeight ?? null,
      unintendedVerticalScroll: Boolean(
        document.documentElement.scrollHeight > document.documentElement.clientHeight + 1
        || (document.body?.scrollHeight ?? 0) > (document.body?.clientHeight ?? 0) + 1
      )
    }
  };
}

function boundWithin(child, parent, tolerance = 1.5) {
  return Boolean(
    child
    && parent
    && child.top >= parent.top - tolerance
    && child.bottom <= parent.bottom + tolerance
  );
}

function assertion(name, passed, detail) {
  return { name, passed, detail };
}

export function evaluateW201CompactGate(measurement, expectedViewport = W201_VIEWPORTS.compact) {
  const assertions = [];
  const add = (name, passed, detail) => assertions.push(assertion(name, passed, detail));
  const bounds = measurement?.bounds ?? {};
  const scroll = measurement?.scrollOwnership ?? {};
  const viewport = measurement?.viewportContract;
  const virtualization = measurement?.virtualization;
  const root = bounds.root;
  const titlebar = bounds.titlebar;
  const workspace = bounds.workspace;
  const workspaceBody = bounds.workspaceBody;
  const contentSlot = bounds.contentSlot;
  const adapter = bounds.legacyLibraryAdapter;
  const viewStage = bounds.viewStage;
  const listbox = bounds.fileLibraryList;
  const listScroll = scroll.fileLibraryList;
  const tolerance = 1.5;
  const compactLibraryClippingDetected = Boolean(
    !workspace
    || !workspaceBody
    || !contentSlot
    || !adapter
    || !listbox
    || !boundWithin(workspace, root, tolerance)
    || !boundWithin(workspaceBody, workspace, tolerance)
    || !boundWithin(contentSlot, workspaceBody, tolerance)
    || !boundWithin(adapter, contentSlot, tolerance)
    || !boundWithin(listbox, root, tolerance)
  );
  const vaultScrollContainerCurrentlyScrollable = Boolean(
    listScroll
    && listScroll.clientHeight > 0
    && listScroll.scrollHeight > listScroll.clientHeight
    && ["auto", "scroll"].includes(listScroll.overflowY)
  );
  const contentReachableWithoutClipping = Boolean(
    listbox
    && listbox.height > 0
    && vaultScrollContainerCurrentlyScrollable
    && virtualization?.mountedRowCount > 0
    && virtualization?.logicalCount > virtualization?.mountedRowCount
  );

  add("viewportContract", Boolean(
    viewport?.matchesRequested
    && viewport.requested?.width === expectedViewport.width
    && viewport.requested?.height === expectedViewport.height
  ), viewport);
  add("workspaceWithinRoot", boundWithin(workspace, root, tolerance), { workspace, root });
  add("workspaceStartsAfterTitlebar", Boolean(workspace && titlebar && workspace.top >= titlebar.bottom - tolerance), { workspace, titlebar });
  add("workspaceBodyWithinWorkspace", boundWithin(workspaceBody, workspace, tolerance), { workspaceBody, workspace });
  add("contentSlotWithinWorkspaceBody", boundWithin(contentSlot, workspaceBody, tolerance), { contentSlot, workspaceBody });
  add("legacyAdapterWithinContentSlot", boundWithin(adapter, contentSlot, tolerance), { adapter, contentSlot });
  add("viewStageBounded", boundWithin(viewStage, root, tolerance), { viewStage, root });
  add("fileLibraryListVisible", Boolean(listbox && listbox.height > 0 && listbox.bottom <= (root?.bottom ?? Infinity) + tolerance), { listbox, root });
  add("fileLibraryListOwnsOverflow", Boolean(
    vaultScrollContainerCurrentlyScrollable
    && scroll.fileLibraryListSelector === "tanstack-virtualizer"
  ), { listScroll, selector: scroll.fileLibraryListSelector });
  add("noUnintendedPageScroll", measurement?.page?.unintendedVerticalScroll === false, measurement?.page);
  add("viewStageDoesNotOwnListOverflow", Boolean(!scroll.viewStage || scroll.viewStage.scrollHeight <= scroll.viewStage.clientHeight + tolerance), scroll.viewStage);
  add("workspaceDoesNotGrowPastViewport", Boolean(!scroll.workspace || scroll.workspace.scrollHeight <= scroll.workspace.clientHeight + tolerance), scroll.workspace);
  add("contentReachableWithoutClipping", contentReachableWithoutClipping, { listbox, listScroll, virtualization });
  add("noCompactLibraryClipping", !compactLibraryClippingDetected, { bounds, ancestorOverflow: scroll.ancestorOverflow });

  const hardAssertionSummary = Object.fromEntries(assertions.map((item) => [item.name, item.passed]));
  hardAssertionSummary.compactLibraryClippingDetected = compactLibraryClippingDetected;
  hardAssertionSummary.vaultScrollContainerCurrentlyScrollable = vaultScrollContainerCurrentlyScrollable;
  hardAssertionSummary.contentReachableWithoutClipping = contentReachableWithoutClipping;
  return {
    passed: assertions.every((item) => item.passed),
    assertions,
    hardAssertionSummary
  };
}

export function evaluateW201ProjectionGate(measurement, expectedViewport, projection) {
  const assertions = [];
  const add = (name, passed, detail) => assertions.push(assertion(name, passed, detail));
  const viewport = measurement?.viewportContract;
  const root = measurement?.bounds?.root;
  const workspace = measurement?.bounds?.workspace;
  const page = measurement?.page;
  add("viewportContract", Boolean(
    viewport?.matchesRequested
    && viewport.requested?.width === expectedViewport.width
    && viewport.requested?.height === expectedViewport.height
  ), viewport);
  add("rootBounded", Boolean(root && root.top >= -1.5 && root.bottom <= expectedViewport.height + 1.5), root);
  add("noUnintendedPageScroll", page?.unintendedVerticalScroll === false, page);
  if (projection === "detached-browse") {
    add("workspaceBounded", Boolean(workspace && workspace.bottom <= root?.bottom + 1.5), { workspace, root });
    add("legacyAdapterAbsent", measurement?.bounds?.legacyLibraryAdapter === null, measurement?.bounds?.legacyLibraryAdapter);
    add("fileLibraryListAbsent", measurement?.bounds?.fileLibraryList === null, measurement?.bounds?.fileLibraryList);
  } else if (projection === "overview") {
    add("fileLibraryWorkspaceAbsent", measurement?.bounds?.workspace === null, measurement?.bounds?.workspace);
    add("legacyAdapterAbsent", measurement?.bounds?.legacyLibraryAdapter === null, measurement?.bounds?.legacyLibraryAdapter);
  }
  return {
    passed: assertions.every((item) => item.passed),
    assertions,
    hardAssertionSummary: Object.fromEntries(assertions.map((item) => [item.name, item.passed]))
  };
}

export function evaluateW201VirtualizationInteraction(before, after) {
  const beforeScroll = before?.scrollOwnership?.fileLibraryList;
  const afterScroll = after?.scrollOwnership?.fileLibraryList;
  const beforeVirtualization = before?.virtualization;
  const afterVirtualization = after?.virtualization;
  const beforeAdapter = before?.scrollOwnership?.legacyLibraryAdapter;
  const afterAdapter = after?.scrollOwnership?.legacyLibraryAdapter;
  const assertions = [
    assertion("scrollTopChanges", Boolean(afterScroll && beforeScroll && afterScroll.scrollTop > beforeScroll.scrollTop), { before: beforeScroll?.scrollTop, after: afterScroll?.scrollTop }),
    assertion("listboxRemainsScrollOwner", Boolean(
      before?.scrollOwnership?.fileLibraryListSelector === "tanstack-virtualizer"
      && after?.scrollOwnership?.fileLibraryListSelector === "tanstack-virtualizer"
      && (!beforeAdapter || !afterAdapter || afterAdapter.scrollTop === beforeAdapter.scrollTop)
    ), {
      selectorBefore: before?.scrollOwnership?.fileLibraryListSelector,
      selectorAfter: after?.scrollOwnership?.fileLibraryListSelector,
      adapterScrollTopBefore: beforeAdapter?.scrollTop,
      adapterScrollTopAfter: afterAdapter?.scrollTop
    }),
    assertion("virtualRangeChanges", Boolean(
      beforeVirtualization
      && afterVirtualization
      && (beforeVirtualization.firstMountedRowIndex !== afterVirtualization.firstMountedRowIndex
        || beforeVirtualization.lastMountedRowIndex !== afterVirtualization.lastMountedRowIndex)
    ), { before: beforeVirtualization, after: afterVirtualization }),
    assertion("mountedRowsRemainBounded", Boolean(
      afterVirtualization
      && afterVirtualization.mountedRowCount > 0
      && afterVirtualization.mountedRowCount < (afterVirtualization.logicalCount ?? Infinity)
      && afterVirtualization.mountedRowCount <= 40
    ), afterVirtualization),
    assertion("loadMoreOrCompletionObserved", Boolean(
      afterVirtualization
      && (
        (afterVirtualization.logicalCount ?? 0) > (beforeVirtualization?.logicalCount ?? 0)
        || afterVirtualization.allResultsLoaded === true
        || afterVirtualization.hasMore === false
      )
    ), { before: beforeVirtualization, after: afterVirtualization })
  ];
  return {
    passed: assertions.every((item) => item.passed),
    assertions,
    scrollOwnershipSummary: {
      before: beforeScroll,
      after: afterScroll,
      virtualRangeBefore: [beforeVirtualization?.firstMountedRowIndex, beforeVirtualization?.lastMountedRowIndex],
      virtualRangeAfter: [afterVirtualization?.firstMountedRowIndex, afterVirtualization?.lastMountedRowIndex]
    }
  };
}

export function evaluateW201ResponsiveGate(measurement, expectedViewport) {
  const assertions = [];
  const add = (name, passed, detail) => assertions.push(assertion(name, passed, detail));
  const bounds = measurement?.bounds ?? {};
  const scroll = measurement?.scrollOwnership ?? {};
  const viewport = measurement?.viewportContract;
  const root = bounds.root;
  const titlebar = bounds.titlebar;
  const workspace = bounds.workspace;
  const listbox = bounds.fileLibraryList;
  const listScroll = scroll.fileLibraryList;
  const tolerance = 1.5;
  const boundedScroll = (metric) => Boolean(
    !metric || metric.scrollHeight <= metric.clientHeight + tolerance
  );
  const resultScrollCandidates = [scroll.resultRegion, scroll.resultMain, scroll.resultSection].filter(Boolean);

  add("viewportContract", Boolean(
    viewport?.matchesRequested
    && viewport.requested?.width === expectedViewport.width
    && viewport.requested?.height === expectedViewport.height
  ), viewport);
  add("workspaceWithinRoot", boundWithin(workspace, root, tolerance), { workspace, root });
  add("workspaceStartsAfterTitlebar", Boolean(workspace && titlebar && workspace.top >= titlebar.bottom - tolerance), { workspace, titlebar });
  add("noUnintendedPageScroll", measurement?.page?.unintendedVerticalScroll === false, measurement?.page);
  add("viewStageBounded", boundedScroll(scroll.viewStage), scroll.viewStage);
  add("workspaceBounded", boundedScroll(scroll.workspace), scroll.workspace);
  add("contentSlotBounded", boundedScroll(scroll.contentSlot), scroll.contentSlot);
  add("legacyAdapterBounded", boundedScroll(scroll.legacyLibraryAdapter), scroll.legacyLibraryAdapter);
  add("vaultRootBounded", boundedScroll(scroll.vaultRoot), scroll.vaultRoot);
  add("fileLibraryListVisible", Boolean(listbox && listbox.height > 0 && boundWithin(listbox, root, tolerance)), { listbox, root });
  add("fileLibraryListOwnsOverflow", Boolean(
    listScroll
    && listScroll.clientHeight > 0
    && listScroll.scrollHeight > listScroll.clientHeight
    && ["auto", "scroll"].includes(listScroll.overflowY)
    && scroll.fileLibraryListSelector === "tanstack-virtualizer"
  ), { listScroll, selector: scroll.fileLibraryListSelector });
  add("noDoubleResultScroll", resultScrollCandidates.every((metric) => metric.scrollHeight <= metric.clientHeight + tolerance), resultScrollCandidates);
  add("legacyAdapterDoesNotScrollResults", Boolean(
    !scroll.legacyLibraryAdapter
    || scroll.legacyLibraryAdapter.scrollHeight <= scroll.legacyLibraryAdapter.clientHeight + tolerance
  ), scroll.legacyLibraryAdapter);
  add("vaultRootDoesNotScrollResults", Boolean(
    !scroll.vaultRoot
    || scroll.vaultRoot.scrollHeight <= scroll.vaultRoot.clientHeight + tolerance
  ), scroll.vaultRoot);

  return {
    passed: assertions.every((item) => item.passed),
    assertions,
    hardAssertionSummary: Object.fromEntries(assertions.map((item) => [item.name, item.passed]))
  };
}
