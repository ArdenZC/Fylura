// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { MountedLocationProjection } from "../src/views/fileLibrary/navigation/mountedLocations";
import type { KnownLocation } from "../src/api/fileManagementApi";
const usb: KnownLocation = { id: "usb", kind: "volume", path: "E:/", displayName: "USB", available: true };
afterEach(() => vi.useRealTimers());
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
  await projection.refresh(); expect(projection.getSnapshot().locations).toEqual([]);
  await projection.refresh(); expect(projection.getSnapshot().locations[0].displayName).toBe("Replacement device");
  await projection.refresh();
  expect(projection.getSnapshot().failed).toBe(true);
  expect(projection.getSnapshot().locations[0].available).toBe(false);
});
