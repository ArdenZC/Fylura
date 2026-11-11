// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { renderToString } from "react-dom/server";
import { MountedLocationProjection, mountedLocations, networkAvailabilityFromBrowseError } from "../src/views/fileLibrary/navigation/mountedLocations";
import { MountedVolumeList } from "../src/views/fileLibrary/navigation/MountedVolumeList";
import type { KnownLocation } from "../src/api/fileManagementApi";
import { fileManagementApi } from "../src/api/fileManagementApi";
import { makeTranslator } from "../src/i18n";
import { browsePlaceStatusLabel, projectKnownLocations, projectMountedNetworkStatuses, useBrowsePlaces } from "../src/views/fileLibrary/browse/useBrowsePlaces";
const usb: KnownLocation = { id: "usb", kind: "volume", path: "E:/", displayName: "USB", available: true };
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });
it("shares one foreground discovery request across panes and stops polling after the last pane closes", async () => {
  vi.useFakeTimers();
  const load = vi.fn().mockResolvedValue([usb, { ...usb, id: "home", kind: "folder" }]);
  const projection = new MountedLocationProjection(load);
  const stopA = projection.subscribe(vi.fn());
  const stopB = projection.subscribe(vi.fn());
  await projection.refresh();
  expect(load).toHaveBeenCalledTimes(1);
  expect(projection.getSnapshot().locations).toEqual([usb]);
  await vi.advanceTimersByTimeAsync(5000);
  expect(load).toHaveBeenCalledTimes(2);
  stopA(); stopB();
  await vi.advanceTimersByTimeAsync(10000);
  window.dispatchEvent(new Event("focus"));
  expect(load).toHaveBeenCalledTimes(2);
});
it("reflects unplug/replug and marks stale discovery unavailable on failure", async () => {
  const load = vi.fn().mockResolvedValueOnce([usb]).mockResolvedValueOnce([])
    .mockResolvedValueOnce([{ ...usb, displayName: "Replacement device" }]).mockRejectedValueOnce(Error("device unavailable"));
  const projection = new MountedLocationProjection(load);
  await projection.refresh();
  await projection.refresh(); expect(projection.getSnapshot().locations).toEqual([{ ...usb, available: false }]);
  await projection.refresh(); expect(projection.getSnapshot().locations[0].displayName).toBe("Replacement device");
  await projection.refresh();
  expect(projection.getSnapshot().failed).toBe(true);
  expect(projection.getSnapshot().locations[0].available).toBe(false);
});
it("binds eject to the displayed volume identity and preserves a system veto without retrying", async () => {
  const device: KnownLocation = { ...usb, volume: { identity: "original-volume", kind: "removable", readOnly: false, availableBytes: 100, totalBytes: 200, canEject: true } };
  const load = vi.fn().mockResolvedValue([device]);
  const eject = vi.spyOn(fileManagementApi, "ejectVolume").mockRejectedValue(Error("storage_eject_busy"));
  const projection = new MountedLocationProjection(load);
  await projection.refresh();
  await projection.eject(device);
  expect(eject).toHaveBeenCalledExactlyOnceWith({ locationId: device.id, identity: "original-volume" });
  expect(projection.getSnapshot().ejectError).toContain("storage_eject_busy");
  expect(projection.getSnapshot().locations[0].available).toBe(true);
  expect(projection.getSnapshot().ejectingId).toBeNull();
});
it("coalesces removal and refresh so an older poll cannot republish an ejected device", async () => {
  const device: KnownLocation = { ...usb, volume: { identity: "original-volume", kind: "removable", readOnly: false, availableBytes: 100, totalBytes: 200, canEject: true } };
  let finish!: (locations: KnownLocation[]) => void;
  const load = vi.fn().mockImplementationOnce(() => new Promise<KnownLocation[]>((resolve) => { finish = resolve; })).mockResolvedValue([]);
  const eject = vi.spyOn(fileManagementApi, "ejectVolume").mockResolvedValue(undefined);
  const projection = new MountedLocationProjection(load);
  const polling = projection.refresh();
  const removal = projection.eject(device);
  const shared = projection.refresh();
  finish([device]);
  await polling; await shared; await removal;
  expect(eject).toHaveBeenCalledTimes(1);
  expect(load).toHaveBeenCalledTimes(2);
  expect(projection.getSnapshot().locations[0].available).toBe(false);
});

it("retains mounted cloud folders and their permission state without requesting accounts or content", async () => {
  const cloud: KnownLocation = { id: "cloud:one", kind: "cloud_provider", path: "C:/Users/test/OneDrive", displayName: "OneDrive", available: false,
    volume: { identity: null, kind: "cloud_provider", readOnly: null, availableBytes: null, totalBytes: null, canEject: false, availability: "permission_required" } };
  const load = vi.fn().mockResolvedValueOnce([cloud]).mockResolvedValueOnce([{ ...cloud, available: true, volume: { ...cloud.volume!, availability: "available" } }]);
  const projection = new MountedLocationProjection(load);
  await projection.refresh();
  expect(projection.getSnapshot().locations).toEqual([cloud]);
  await projection.refresh();
  expect(projection.getSnapshot().locations[0]).toMatchObject({ id: cloud.id, available: true, volume: { canEject: false, availability: "available" } });
});

it("shows connected unlettered SMB shares and preserves disconnect/reconnect feedback", async () => {
  const smb: KnownLocation = {
    id: "volume:\\\\nas.example\\documents",
    kind: "volume",
    path: "\\\\nas.example\\documents",
    displayName: "\\\\nas.example\\documents",
    available: false,
    volume: { identity: null, kind: "network", readOnly: null, availableBytes: null, totalBytes: null, canEject: false, availability: "connected_unverified" }
  };
  const load = vi.fn().mockResolvedValueOnce([smb]).mockResolvedValueOnce([]).mockResolvedValueOnce([smb]);
  const projection = new MountedLocationProjection(load);
  await projection.refresh();
  expect(projection.getSnapshot().locations).toEqual([smb]);
  await projection.refresh();
  expect(projection.getSnapshot().locations[0]).toMatchObject({ path: smb.path, available: false, volume: { availability: "offline" } });
  await projection.refresh();
  expect(projection.getSnapshot().locations[0]).toMatchObject({ path: smb.path, available: false, volume: { kind: "network", canEject: false, availability: "connected_unverified" } });
  expect(load).toHaveBeenCalledTimes(3);
});

it("does not apply network Browse reports to a local mounted volume", async () => {
  const local: KnownLocation = {
    id: "volume:C:",
    kind: "volume",
    path: "C:/",
    displayName: "System drive",
    available: true,
    volume: { identity: "local-volume", kind: "local", readOnly: false, availableBytes: 10, totalBytes: 20, canEject: false, availability: "available" }
  };
  const projection = new MountedLocationProjection(vi.fn().mockResolvedValue([local]));
  await projection.refresh();

  projection.recordNetworkBrowse(local.id, local.path, "offline", "local-browse-session");
  projection.recordNetworkBrowseSessionResult("local-browse-session", "offline");
  expect(projection.getSnapshot().locations).toEqual([local]);
});

it("verifies SMB only after an explicit Browse enumeration and resets stale failures for a manual retry", async () => {
  const smb: KnownLocation = {
    id: "volume:\\\\nas.example\\documents",
    kind: "volume",
    path: "\\\\nas.example\\documents",
    displayName: "\\\\nas.example\\documents",
    available: false,
    volume: { identity: null, kind: "network", readOnly: null, availableBytes: null, totalBytes: null, canEject: false, availability: "connected_unverified" }
  };
  const load = vi.fn().mockResolvedValue([smb]);
  const projection = new MountedLocationProjection(load);
  await projection.refresh();
  projection.recordNetworkBrowse(smb.id, smb.path, "connected_unverified", "session-offline");
  projection.recordNetworkBrowseSessionResult("session-offline", "offline");
  expect(projection.getSnapshot().locations[0]).toMatchObject({ available: false, volume: { availability: "offline" } });

  // Passive discovery does not test the remote path and keeps the explicit
  // last-open failure until the user requests a refresh/retry.
  await projection.refresh();
  expect(projection.getSnapshot().locations[0]).toMatchObject({ available: false, volume: { availability: "offline" } });

  await projection.refresh(true);
  expect(projection.getSnapshot().locations[0]).toMatchObject({ available: false, volume: { availability: "connected_unverified" } });
  projection.recordNetworkBrowseSessionResult("session-offline", "permission_required");
  expect(projection.getSnapshot().locations[0]).toMatchObject({ available: false, volume: { availability: "connected_unverified" } });
  projection.recordNetworkBrowse(smb.id, smb.path, "connected_unverified", "session-reconnected");
  projection.recordNetworkBrowseSessionResult("session-reconnected", "available");
  expect(projection.getSnapshot().locations[0]).toMatchObject({ available: true, volume: { availability: "available" } });
});

it("resets mounted SMB observations when the Browse Places refresh action is used", async () => {
  vi.spyOn(fileManagementApi, "fileLibraryKnownLocations").mockResolvedValue([]);
  vi.spyOn(fileManagementApi, "fileLibraryPlaces").mockResolvedValue([]);
  const refresh = vi.spyOn(mountedLocations, "refresh").mockResolvedValue(undefined);
  let reload: () => Promise<void> = async () => {};
  function Harness() {
    reload = useBrowsePlaces({ enabled: false, openPath: async () => false, t: makeTranslator("en") }).reload;
    return null;
  }
  const root = createRoot(document.createElement("div"));
  await act(async () => { root.render(createElement(Harness)); });
  await act(async () => { await reload(); });
  expect(refresh).toHaveBeenCalledWith(true);
  await act(async () => { root.unmount(); });
});

it("ignores out-of-order SMB results from older panes while the newest of four panes stays authoritative", async () => {
  const smb: KnownLocation = {
    id: "volume:\\\\nas.example\\documents",
    kind: "volume",
    path: "\\\\nas.example\\documents",
    displayName: "NAS documents",
    available: false,
    volume: { identity: null, kind: "network", readOnly: null, availableBytes: null, totalBytes: null, canEject: false, availability: "connected_unverified" }
  };
  const projection = new MountedLocationProjection(vi.fn().mockResolvedValue([smb]));
  await projection.refresh();

  projection.recordNetworkBrowse(smb.id, smb.path, "connected_unverified", "pane-session-1");
  projection.recordNetworkBrowse(smb.id, smb.path, "connected_unverified", "pane-session-2");
  projection.recordNetworkBrowse(smb.id, smb.path, "connected_unverified", "pane-session-3");
  projection.recordNetworkBrowse(smb.id, smb.path, "connected_unverified", "pane-session-4");
  projection.recordNetworkBrowseSessionResult("pane-session-4", "available");
  projection.recordNetworkBrowseSessionResult("pane-session-1", "offline");
  projection.recordNetworkBrowseSessionResult("pane-session-2", "permission_required");
  projection.recordNetworkBrowseSessionResult("pane-session-3", "offline");
  expect(projection.getSnapshot().locations[0]).toMatchObject({ available: true, volume: { availability: "available" } });
  projection.recordNetworkBrowseSessionResult("pane-session-4", "offline");
  expect(projection.getSnapshot().locations[0]).toMatchObject({ available: false, volume: { availability: "offline" } });
  projection.recordNetworkBrowseSessionResult("pane-session-4", "available");
  expect(projection.getSnapshot().locations[0]).toMatchObject({ available: true, volume: { availability: "available" } });

  // A later attempt can fail before admission returns a session id. That
  // explicit result also supersedes any still-running Browse enumeration.
  projection.recordNetworkBrowse(smb.id, smb.path, "permission_required");
  projection.recordNetworkBrowseSessionResult("pane-session-4", "available");
  expect(projection.getSnapshot().locations[0]).toMatchObject({ available: false, volume: { availability: "permission_required" } });
});

it("keeps a newer explicit SMB result when a slower discovery request returns stale state", async () => {
  const smb: KnownLocation = {
    id: "volume:\\\\nas.example\\documents", kind: "volume", path: "\\\\nas.example\\documents", displayName: "NAS documents", available: false,
    volume: { identity: null, kind: "network", readOnly: null, availableBytes: null, totalBytes: null, canEject: false, availability: "connected_unverified" }
  };
  const staleResponses: KnownLocation[][] = [
    [{ ...smb, volume: { ...smb.volume!, availability: "offline" } }],
    [{ ...smb, volume: { ...smb.volume!, availability: "permission_required" } }],
    []
  ];
  for (const staleResponse of staleResponses) {
    let finish!: (locations: KnownLocation[]) => void;
    const load = vi.fn()
      .mockResolvedValueOnce([smb])
      .mockImplementationOnce(() => new Promise<KnownLocation[]>((resolve) => { finish = resolve; }));
    const projection = new MountedLocationProjection(load);
    await projection.refresh();

    const refreshStartedBeforeOpen = projection.refresh();
    projection.recordNetworkBrowse(smb.id, smb.path, "connected_unverified", "newer-session");
    projection.recordNetworkBrowseSessionResult("newer-session", "available");
    finish(staleResponse);
    await refreshStartedBeforeOpen;

    expect(projection.getSnapshot().locations[0]).toMatchObject({ available: true, volume: { availability: "available" } });
  }
});

it("renders a connected unlettered SMB share as an openable network location", () => {
  const smb: KnownLocation = {
    id: "volume:\\\\nas.example\\documents",
    kind: "volume",
    path: "\\\\nas.example\\documents",
    displayName: "\\\\nas.example\\documents",
    available: false,
    volume: { identity: null, kind: "network", readOnly: null, availableBytes: null, totalBytes: null, canEject: false, availability: "connected_unverified" }
  };
  const onOpen = vi.fn();
  const html = renderToString(createElement(MountedVolumeList, {
    mounted: {
      locations: [smb], loading: false, failed: false, ejectingId: null, ejectError: null,
      refresh: async () => undefined, recordNetworkBrowse: () => undefined, eject: async () => undefined
    },
    onOpen,
    t: makeTranslator("en")
  }));
  expect(html).toContain('title="\\\\nas.example\\documents"');
  expect(html).toContain('aria-hidden="true"');
  expect(html).toContain("Connection recorded by the system; open to check access");
  expect(html).not.toContain("disabled=\"\"");
  expect(onOpen).not.toHaveBeenCalled();
});

it("keeps connected SMB Places openable without presenting it as verified online", () => {
  const smb: KnownLocation = {
    id: "volume:Z:", kind: "volume", path: "Z:/", displayName: "Z:", available: false,
    volume: { identity: null, kind: "network", readOnly: null, availableBytes: null, totalBytes: null, canEject: false, availability: "connected_unverified" }
  };
  const [place] = projectKnownLocations([smb], makeTranslator("en"));
  expect(place).toMatchObject({ openable: true, network: true, availability: "connected_unverified", locationId: smb.id });
  expect(browsePlaceStatusLabel(place!, makeTranslator("en"))).toBe("Connection recorded by the system; open to check access");
});

it("shows the latest mounted SMB browse result in Places as well as the sidebar", () => {
  const smb: KnownLocation = {
    id: "volume:\\\\nas.example\\documents", kind: "volume", path: "\\\\nas.example\\documents", displayName: "NAS documents", available: false,
    volume: { identity: null, kind: "network", readOnly: null, availableBytes: null, totalBytes: null, canEject: false, availability: "connected_unverified" }
  };
  const [entry] = projectKnownLocations([smb], makeTranslator("en"));
  const latest = { ...smb, volume: { ...smb.volume!, availability: "permission_required" as const } };
  const [projected] = projectMountedNetworkStatuses([entry!], [latest]);
  expect(projected).toMatchObject({ openable: true, availability: "permission_required" });
  expect(browsePlaceStatusLabel(projected!, makeTranslator("en"))).toBe("Permission required. Connect and authorize in the system or cloud client");
});

it("classifies explicit SMB Browse failures without probing network paths", () => {
  expect(networkAvailabilityFromBrowseError("browse_directory_permission_denied")).toBe("permission_required");
  expect(networkAvailabilityFromBrowseError("browse_directory_offline")).toBe("offline");
  expect(networkAvailabilityFromBrowseError("network_unreachable")).toBe("offline");
  expect(networkAvailabilityFromBrowseError("browse_directory_not_found")).toBe("unknown");
  expect(networkAvailabilityFromBrowseError("browse_directory_unavailable")).toBe("unknown");
  expect(networkAvailabilityFromBrowseError("browse_cancelled")).toBeNull();
  expect(networkAvailabilityFromBrowseError("browse_enumeration_stale")).toBeNull();
  expect(networkAvailabilityFromBrowseError("browse_cursor_invalid")).toBeNull();
  expect(networkAvailabilityFromBrowseError({ message: "browse_directory_permission_denied" })).toBe("permission_required");
});
