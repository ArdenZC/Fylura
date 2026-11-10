// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { MountedLocationProjection } from "../src/views/fileLibrary/navigation/mountedLocations";
import type { KnownLocation } from "../src/api/fileManagementApi";
import { fileManagementApi } from "../src/api/fileManagementApi";
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
