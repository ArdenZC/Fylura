// @vitest-environment happy-dom

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { act, createElement, StrictMode, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import {
  FileWorkspaceController,
  WorkspaceSession,
  type FileWorkspaceApi
} from "../src/fileWorkspace";
import { makeTranslator } from "../src/i18n";
import { WorkspaceCommandBar } from "../src/views/fileLibrary/FileLibraryWorkspace";
import { FileLibraryExperienceProvider, useFileLibraryExperience } from "../src/views/fileLibrary/FileLibraryExperienceProvider";
import {
  FileLibraryExperienceController,
  LEGACY_LIBRARY_MIGRATION_TARGET,
  isManagedLibraryBrowserFixture
} from "../src/views/fileLibrary/fileLibraryExperience";
import type { FileLibraryExperienceState } from "../src/views/fileLibrary/fileLibraryExperience";
import type { BrowseOpenResponse, BrowsePage, LocationRef } from "../src/types/fileWorkspace";

const t = makeTranslator("zh");

function fakeResponse(sessionId = "browse-session"): BrowseOpenResponse {
  return {
    sessionId,
    location: {
      ref: { kind: "ephemeral", browseSessionId: sessionId, locationId: "location" },
      displayName: "Documents",
      kind: "local",
      availability: "available",
      freshness: "current",
      capabilities: {
        canBrowse: true,
        canReadMetadata: true,
        canPreview: true,
        canWatch: true,
        canRequestMaterialization: false,
        canAddToLibrary: true
      }
    },
    rootPathRef: { id: "root" }
  };
}

function fakeApi(overrides: Partial<FileWorkspaceApi> = {}): FileWorkspaceApi {
  const page: BrowsePage = {
    sessionId: "browse-session",
    requestId: "enumeration-request",
    enumerationId: "enumeration",
    entries: [],
    completion: "complete"
  };
  return {
    browseOpen: async () => fakeResponse(),
    browseRestore: async () => { throw new Error("restore should not be used"); },
    locationBrowse: async () => { throw new Error("location browse should not be used"); },
    browseStartEnumeration: async () => page,
    browseNextPage: async () => page,
    browseCancel: async () => undefined,
    browseReleasePage: async () => undefined,
    browseReleasePath: async () => undefined,
    browseRetainPath: async () => undefined,
    browseDispose: async () => undefined,
    locationList: async () => [],
    changeStart: async ({ sessionId, pathRef }) => ({ monitorId: "monitor", sessionId, pathRef }),
    changePending: async () => ({ monitorId: "monitor", sequence: 1, hint: { kind: "content_changed" } }),
    changeRefresh: async () => page,
    changeDispose: async () => undefined,
    readEligibility: async ({ source }) => ({ source, eligibility: "eligible" }),
    thumbnailRequest: async () => ({ cacheKey: "thumbnail", bytes: new Uint8Array() }),
    thumbnailCancel: async () => true,
    previewCreate: async () => { throw new Error("preview should not be used"); },
    previewSnapshot: async () => { throw new Error("preview should not be used"); },
    previewStart: async () => { throw new Error("preview should not be used"); },
    previewCancel: async () => true,
    previewDispose: async () => true,
    previewSwitchSource: async () => { throw new Error("preview should not be used"); },
    previewAssetRequest: async () => { throw new Error("preview asset should not be used"); },
    ...overrides
  };
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

async function settleLifecycle() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
  await act(async () => {
    await new Promise<void>((resolvePromise) => setTimeout(resolvePromise, 0));
  });
}

function experienceWithApi(overrides: Partial<FileWorkspaceApi> = {}) {
  const session = new WorkspaceSession({ initialTarget: LEGACY_LIBRARY_MIGRATION_TARGET });
  const workspace = new FileWorkspaceController(fakeApi(overrides), session);
  return { experience: new FileLibraryExperienceController(workspace), workspace, session };
}

describe("W2-01 File Library Experience Controller", () => {
  it("starts the ordinary Files workspace in Browse without a Managed Library target", async () => {
    const experience = new FileLibraryExperienceController(new FileWorkspaceController(fakeApi()));

    expect(experience.getState().mode).toBe("browse");
    expect(experience.getState().detachedBrowse).toBe(true);
    expect(experience.getState().workspace.session.currentTarget).toBeNull();
    expect(experience.getState().workspace.session.history).toHaveLength(0);
    await experience.dispose();
  });

  it("keeps first-entry Browse detached and non-authoritative", async () => {
    const browseOpen = vi.fn(async () => fakeResponse());
    const { experience, session } = experienceWithApi({ browseOpen });

    await expect(experience.switchMode("browse")).resolves.toBe(true);
    expect(experience.getState().mode).toBe("browse");
    expect(experience.getState().detachedBrowse).toBe(true);
    expect(session.getState().currentTarget).toEqual(LEGACY_LIBRARY_MIGRATION_TARGET);
    expect(session.getState().history).toHaveLength(1);
    expect(session.getState().lastBrowseTarget).toBeNull();
    expect(browseOpen).not.toHaveBeenCalled();

    await expect(experience.switchMode("library")).resolves.toBe(true);
    expect(experience.getState().mode).toBe("library");
    expect(experience.getState().detachedBrowse).toBe(false);
    await experience.dispose();
  });

  it("retains detached Browse projection across unrelated workspace emissions until admission", async () => {
    const browseOpen = vi.fn(async () => fakeResponse("admitted-browse-session"));
    const { experience, workspace, session } = experienceWithApi({ browseOpen });

    await expect(experience.switchMode("browse")).resolves.toBe(true);
    const before = session.getState();

    await workspace.loadLocations();

    expect(experience.getState().mode).toBe("browse");
    expect(experience.getState().detachedBrowse).toBe(true);
    expect(session.getState().currentTarget).toEqual(before.currentTarget);
    expect(session.getState().history).toEqual(before.history);
    expect(session.getState().lastBrowseTarget).toEqual(before.lastBrowseTarget);

    const admitted = await experience.openBrowse({ platform: "windows", routingHint: "Documents" });
    expect(admitted?.sessionId).toBe("admitted-browse-session");
    expect(experience.getState().mode).toBe("browse");
    expect(experience.getState().detachedBrowse).toBe(false);
    expect(session.getState().currentTarget?.kind).toBe("browse");

    await experience.dispose();
  });

  it("routes remembered mode switching through controller cleanup", async () => {
    const browseDispose = vi.fn(async () => undefined);
    const browseReleasePage = vi.fn(async () => undefined);
    const changeDispose = vi.fn(async () => undefined);
    const { experience, workspace } = experienceWithApi({ browseDispose, browseReleasePage, changeDispose });

    const opened = await experience.openBrowse({ platform: "windows", routingHint: "Documents" });
    expect(opened).not.toBeNull();
    await workspace.startEnumeration(undefined, "enumeration-request", 10);
    await workspace.startChange({ id: "root" });

    await expect(experience.switchMode("library")).resolves.toBe(true);
    expect(experience.getState().mode).toBe("library");
    expect(experience.getState().workspace.browse).toBeNull();
    expect(changeDispose).toHaveBeenCalledWith({ monitorId: "monitor" });
    expect(browseReleasePage).toHaveBeenCalledWith({ page: expect.anything() });
    expect(browseDispose).not.toHaveBeenCalled();

    await expect(experience.switchMode("browse")).resolves.toBe(true);
    expect(experience.getState().mode).toBe("browse");
    expect(experience.getState().detachedBrowse).toBe(false);
    expect(experience.getState().workspace.browse?.sessionId).toBe(opened?.sessionId);
    await experience.dispose();
    expect(browseDispose).toHaveBeenCalledWith({ sessionId: opened?.sessionId });
  });

  it("routes opaque Location admission without creating restore metadata", async () => {
    const location: LocationRef = { kind: "managed", scanRootId: "managed-root" };
    const locationBrowse = vi.fn(async () => fakeResponse("location-admitted"));
    const { experience, session } = experienceWithApi({ locationBrowse });

    const admitted = await experience.browseLocation(location);

    expect(locationBrowse).toHaveBeenCalledWith({ location });
    expect(admitted?.sessionId).toBe("location-admitted");
    expect(experience.getState().mode).toBe("browse");
    expect(experience.getState().detachedBrowse).toBe(false);
    expect(session.serializeRestoreLocator()).toBeNull();

    await experience.dispose();
  });

  it("suspends disposable work while retaining W1 history/session references", async () => {
    const browseReleasePage = vi.fn(async () => undefined);
    const browseDispose = vi.fn(async () => undefined);
    const { experience, workspace, session } = experienceWithApi({ browseReleasePage, browseDispose });

    const opened = await experience.openBrowse({ platform: "windows", routingHint: "Documents" });
    await workspace.startEnumeration(undefined, "suspend-enumeration", 10);
    const before = session.getState();

    await expect(experience.suspend()).resolves.toBe(true);
    const suspended = experience.getState();
    expect(suspended.workspace.suspended).toBe(true);
    expect(suspended.workspace.browse).toBeNull();
    expect(suspended.workspace.page).toBeNull();
    expect(suspended.mode).toBe("browse");
    expect(suspended.detachedBrowse).toBe(false);
    expect(suspended.workspace.session.currentTarget).toEqual(before.currentTarget);
    expect(suspended.workspace.session.history).toEqual(before.history);
    expect(suspended.workspace.session.lastBrowseTarget).toEqual(before.lastBrowseTarget);
    expect(browseReleasePage).toHaveBeenCalledWith({ page: expect.anything() });
    expect(browseDispose).not.toHaveBeenCalled();

    await expect(experience.resume()).resolves.toBe(true);
    expect(experience.getState().workspace.suspended).toBe(false);
    expect(experience.getState().workspace.browse?.sessionId).toBe(opened?.sessionId);

    await experience.dispose();
    expect(browseDispose).toHaveBeenCalledWith({ sessionId: opened?.sessionId });
  });
});

describe("W2-01 File Library Workspace shell contract", () => {
  it("presents one Files mode and hides the migration key", () => {
    const html = renderToStaticMarkup(createElement(WorkspaceCommandBar, {
      mode: "browse",
      targetLabel: t("fileLibraryNoFolderOpen"),
      canGoBack: false,
      canGoForward: false,
      onBack: vi.fn(),
      onForward: vi.fn(),
      t
    }));

    expect(html).toContain("未打开文件夹");
    expect(html).not.toContain("legacy_library");
    expect(html).toContain('data-file-library-nav-toggle="true"');
    expect(html).toContain('aria-controls="file-library-navigation-slot"');
    // ADR-0016: no user-visible Library/Browse tablist in ordinary Files.
    expect(html).not.toContain("file-library-mode-switch");
    expect(html).not.toContain('role="tablist"');
    expect(html).not.toContain('data-file-library-mode="library"');
    expect(html).toContain('data-file-library-presented-mode="browse"');
  });

  it("wires Library through the W2-03 source owner and preserves the standalone Vault seam", () => {
    const shell = readFileSync(resolve("src/components/AppShell.tsx"), "utf8");
    const workspace = readFileSync(resolve("src/views/fileLibrary/FileLibraryWorkspace.tsx"), "utf8");
    const vault = readFileSync(resolve("src/views/vault/VaultView.tsx"), "utf8");
    const list = readFileSync(resolve("src/views/vault/components/FileLibraryList.tsx"), "utf8");

    expect(shell).toContain("FileLibraryExperienceProvider");
    expect(shell).toContain("FileLibraryWorkspace");
    expect(shell).not.toContain("const VaultView = lazy");
    expect(workspace).toContain('data-library-migration-adapter="library-source-owner"');
    expect(workspace).toContain('import("./library/LibraryMode")');
    expect(workspace).not.toContain('import("../vault/VaultView")');
    expect(workspace).not.toContain('presentation="embedded"');
    expect(vault).toContain('presentation = "standalone"');
    expect(vault).toContain("vault-view-embedded-chrome");
    expect(vault).toContain("vault-view-embedded-result-region");
    expect(list).toContain("getScrollElement: () => parentRef.current");
    expect(list).toContain('data-file-library-scroll-owner="tanstack-virtualizer"');
    expect(list).toContain("getPresentationEntry");
    expect(workspace).toContain('data-workspace-slot="navigation"');
    expect(workspace).not.toContain("data-navigation-drawer-layer");
    expect(workspace).not.toContain("file-library-navigation-trigger");
    expect(workspace).not.toContain("WorkspaceNavigation");
    expect(workspace).not.toContain("useFileLibraryStore");
    expect(workspace).not.toContain("Query V2");
    expect(workspace).not.toContain("legacy_library");
  });

  it("suspends hidden workbench panes and guards hidden Browse enumeration", () => {
    const workbench = readFileSync(resolve("src/views/fileLibrary/workbench/FileLibraryWorkbench.tsx"), "utf8");
    const browseOwner = readFileSync(resolve("src/views/fileLibrary/browse/browseSourceOwner.ts"), "utf8");

    expect(workbench).toContain("active={props.visible}");
    expect(workbench).toContain("wideDualPane={tab.dual && wide}");
    expect(browseOwner).toContain("if (!visible || state.workspace.suspended) {");
    expect(browseOwner).toContain("activeEnumerationKeyRef.current = null;");
    expect(browseOwner).toContain("if (state.mode !== \"browse\" || targetKey === null");
  });
});

function ExperienceProbe({ onCapture }: { onCapture: (value: { controller: FileLibraryExperienceController; state: FileLibraryExperienceState }) => void }) {
  onCapture(useFileLibraryExperience());
  return null;
}

function experienceProviderChild(onCapture: (value: { controller: FileLibraryExperienceController; state: FileLibraryExperienceState }) => void): ReactNode {
  return createElement(ExperienceProbe, { onCapture });
}

describe("W2-01 AppShell-lifetime provider", () => {
  it("keeps one controller owner across inactive and active route projections", async () => {
    const controller = new FileLibraryExperienceController(
      new FileWorkspaceController(
        fakeApi(),
        new WorkspaceSession({ initialTarget: LEGACY_LIBRARY_MIGRATION_TARGET })
      )
    );
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    let captured: { controller: FileLibraryExperienceController; state: FileLibraryExperienceState } | undefined;
    const onCapture = (value: { controller: FileLibraryExperienceController; state: FileLibraryExperienceState }) => {
      captured = value;
    };

    await act(async () => {
      root.render(createElement(FileLibraryExperienceProvider, {
        active: true,
        controller,
        children: experienceProviderChild(onCapture)
      }));
    });
    expect(captured?.controller).toBe(controller);

    await act(async () => {
      root.render(createElement(FileLibraryExperienceProvider, {
        active: false,
        controller,
        children: experienceProviderChild(onCapture)
      }));
    });
    expect(controller.workspace.getState().suspended).toBe(true);

    await act(async () => {
      root.render(createElement(FileLibraryExperienceProvider, {
        active: true,
        controller,
        children: experienceProviderChild(onCapture)
      }));
    });
    expect(captured?.controller).toBe(controller);
    expect(controller.workspace.getState().suspended).toBe(false);

    await act(async () => root.unmount());
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 0));
    expect(controller.workspace.session.disposed).toBe(true);
    container.remove();
  });

  it("keeps the owner active when delayed suspend cleanup meets rapid inactive/active", async () => {
    const changeDisposeGate = deferred<void>();
    const changeDispose = vi.fn(() => changeDisposeGate.promise);
    const { experience, workspace, session } = experienceWithApi({ changeDispose });
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    await act(async () => {
      root.render(createElement(FileLibraryExperienceProvider, {
        active: true,
        controller: experience,
        children: experienceProviderChild(() => undefined)
      }));
    });
    const opened = await experience.openBrowse({ platform: "windows", routingHint: "Documents" });
    await workspace.startChange({ id: "root" });
    const before = session.getState();

    await act(async () => {
      root.render(createElement(FileLibraryExperienceProvider, {
        active: false,
        controller: experience,
        children: experienceProviderChild(() => undefined)
      }));
    });
    await act(async () => {
      root.render(createElement(FileLibraryExperienceProvider, {
        active: true,
        controller: experience,
        children: experienceProviderChild(() => undefined)
      }));
    });

    expect(workspace.getState().suspended).toBe(true);
    expect(workspace.getState().browse).toBeNull();
    expect(session.getState().history).toEqual(before.history);
    expect(session.getState().lastBrowseTarget).toEqual(before.lastBrowseTarget);
    expect(changeDispose).toHaveBeenCalledTimes(1);

    await act(async () => {
      changeDisposeGate.resolve();
      await changeDisposeGate.promise;
    });
    await settleLifecycle();

    expect(workspace.getState().suspended).toBe(false);
    expect(workspace.getState().browse?.sessionId).toBe(opened?.sessionId);
    expect(changeDispose).toHaveBeenCalledTimes(1);

    await act(async () => root.unmount());
    await settleLifecycle();
    container.remove();
  });

  it("ends suspended when delayed resume meets rapid active/inactive", async () => {
    const changeDisposeGate = deferred<void>();
    const changeDispose = vi.fn(() => changeDisposeGate.promise);
    const { experience, workspace, session } = experienceWithApi({ changeDispose });
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    await act(async () => {
      root.render(createElement(FileLibraryExperienceProvider, {
        active: true,
        controller: experience,
        children: experienceProviderChild(() => undefined)
      }));
    });
    await experience.openBrowse({ platform: "windows", routingHint: "Documents" });
    await workspace.startChange({ id: "root" });
    const before = session.getState();

    await act(async () => {
      root.render(createElement(FileLibraryExperienceProvider, {
        active: false,
        controller: experience,
        children: experienceProviderChild(() => undefined)
      }));
    });
    await act(async () => {
      root.render(createElement(FileLibraryExperienceProvider, {
        active: true,
        controller: experience,
        children: experienceProviderChild(() => undefined)
      }));
    });
    await act(async () => {
      root.render(createElement(FileLibraryExperienceProvider, {
        active: false,
        controller: experience,
        children: experienceProviderChild(() => undefined)
      }));
    });

    expect(workspace.getState().suspended).toBe(true);
    expect(workspace.getState().browse).toBeNull();
    expect(session.getState().history).toEqual(before.history);

    await act(async () => {
      changeDisposeGate.resolve();
      await changeDisposeGate.promise;
    });
    await settleLifecycle();

    expect(workspace.getState().suspended).toBe(true);
    expect(workspace.getState().browse).toBeNull();
    expect(session.getState().history).toEqual(before.history);
    expect(session.getState().lastBrowseTarget).toEqual(before.lastBrowseTarget);
    expect(changeDispose).toHaveBeenCalledTimes(1);

    await act(async () => root.unmount());
    await settleLifecycle();
    container.remove();
  });

  it("keeps StrictMode replay from disposing the owner and cleans up exactly once", async () => {
    const browseDispose = vi.fn(async () => undefined);
    const workspaceDispose = vi.spyOn(FileWorkspaceController.prototype, "dispose");
    const { experience, workspace } = experienceWithApi({ browseDispose });
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    await act(async () => {
      root.render(createElement(StrictMode, null,
        createElement(FileLibraryExperienceProvider, {
          active: true,
          controller: experience,
          children: experienceProviderChild(() => undefined)
        })
      ));
    });
    await experience.openBrowse({ platform: "windows", routingHint: "Documents" });
    await settleLifecycle();
    expect(workspaceDispose).toHaveBeenCalledTimes(0);
    expect(workspace.session.disposed).toBe(false);

    await act(async () => root.unmount());
    await settleLifecycle();
    expect(workspaceDispose).toHaveBeenCalledTimes(1);
    expect(workspace.session.disposed).toBe(true);
    expect(browseDispose).toHaveBeenCalledTimes(1);

    await experience.dispose();
    expect(workspaceDispose).toHaveBeenCalledTimes(1);
    expect(browseDispose).toHaveBeenCalledTimes(1);
    workspaceDispose.mockRestore();
    container.remove();
  });

  it("revokes pre-suspend tokens while retaining chronology, presentation, and Browse refs", async () => {
    const browseDispose = vi.fn(async () => undefined);
    const { experience, workspace, session } = experienceWithApi({ browseDispose });
    const opened = await experience.openBrowse({ platform: "windows", routingHint: "Documents" });
    expect(opened).not.toBeNull();
    expect(session.setPresentation({ viewMode: "list", scrollAnchor: "root" })).toBe(true);
    const token = session.beginRequest();
    const before = session.getState();

    await expect(experience.suspend()).resolves.toBe(true);
    expect(session.canPublish(token)).toBe(false);
    expect(session.getState().currentTarget).toEqual(before.currentTarget);
    expect(session.getState().history).toEqual(before.history);
    expect(session.getState().lastBrowseTarget).toEqual(before.lastBrowseTarget);
    expect(session.getState().presentation).toEqual(before.presentation);
    expect(workspace.getState().browse).toBeNull();
    expect(browseDispose).not.toHaveBeenCalled();

    await expect(experience.resume()).resolves.toBe(true);
    expect(workspace.getState().browse?.sessionId).toBe(opened?.sessionId);
    expect(workspace.getState().browse?.rootPathRef.id).toBe(before.lastBrowseTarget?.pathRef.id);
    expect(session.getState().presentation).toEqual(before.presentation);

    await experience.dispose();
    expect(browseDispose).toHaveBeenCalledTimes(1);
  });
});


describe("Managed Library browser compatibility boundary", () => {
  it("requires explicit fixture opt-in and rejects a native Tauri runtime", () => {
    const original = window.location.href;
    try {
      window.history.replaceState(null, "", "/");
      expect(isManagedLibraryBrowserFixture()).toBe(false);
      window.history.replaceState(null, "", "/?managed-library-test=1");
      expect(isManagedLibraryBrowserFixture()).toBe(true);
      const controller = new FileLibraryExperienceController();
      expect(controller.getState().mode).toBe("library");
      expect(controller.getState().workspace.session.currentTarget).toEqual(LEGACY_LIBRARY_MIGRATION_TARGET);
      void controller.dispose();
      vi.stubGlobal("__TAURI__", {});
      expect(isManagedLibraryBrowserFixture()).toBe(false);
      const native = new FileLibraryExperienceController();
      expect(native.getState().mode).toBe("browse");
      expect(native.getState().workspace.session.currentTarget).toBeNull();
      void native.dispose();
    } finally {
      vi.unstubAllGlobals();
      window.history.replaceState(null, "", original);
    }
  });
});
