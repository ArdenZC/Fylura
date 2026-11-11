// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToString } from "react-dom/server";
import { MountedLocationProjection } from "../src/views/fileLibrary/navigation/mountedLocations";
import { networkAvailabilityFromBrowseError } from "../src/views/fileLibrary/navigation/mountedLocations";
import { MountedVolumeList } from "../src/views/fileLibrary/navigation/MountedVolumeList";
import type { KnownLocation } from "../src/api/fileManagementApi";
import { fileManagementApi } from "../src/api/fileManagementApi";
import { makeTranslator } from "../src/i18n";
import { browsePlaceStatusLabel, projectKnownLocations } from "../src/views/fileLibrary/browse/useBrowsePlaces";
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
  expect(projection.projectKnownLocation(smb)).toMatchObject({ available: false, volume: { availability: "offline" } });

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

it("classifies explicit SMB Browse failures without probing network paths", () => {
  expect(networkAvailabilityFromBrowseError("browse_directory_permission_denied")).toBe("permission_required");
  expect(networkAvailabilityFromBrowseError("browse_directory_offline")).toBe("offline");
  expect(networkAvailabilityFromBrowseError("browse_directory_not_found")).toBe("offline");
  expect(networkAvailabilityFromBrowseError("browse_directory_unavailable")).toBe("unknown");
});
