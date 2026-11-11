import { Cloud, Unplug, HardDrive, Network } from "lucide-react";
import type { KnownLocation } from "../../../api/fileManagementApi";
import type { Translator } from "../../../types/ui";
import { formatBytes } from "../../../utils/format";
import { useFileManagementStore } from "../management/fileManagementStore";
import type { useMountedLocations } from "./mountedLocations";

export function MountedVolumeList({ mounted, onOpen, t }: {
  mounted: ReturnType<typeof useMountedLocations>;
  onOpen: (location: KnownLocation) => void;
  t: Translator;
}) {
  const transferring = useFileManagementStore((state) => state.transferActive);
  const errorKey = mounted.ejectError?.includes("storage_eject_busy") ? "filesEjectBusy"
    : /storage_device_(changed|disconnected)/.test(mounted.ejectError ?? "") ? "filesEjectChanged" : "filesEjectUnsupported";
  return <>
    {mounted.ejectError ? <p role="status" className="file-library-navigation-muted">{t(errorKey)}</p> : null}
    {mounted.locations.map((location) => {
      const Icon = location.kind === "cloud_provider" ? Cloud : location.volume?.kind === "network" ? Network : HardDrive;
      const networkLocation = location.volume?.kind === "network";
      const networkAvailability = location.volume?.availability ?? "unknown";
      const networkStatusKey = networkAvailability === "connected_unverified" ? "filesLocationConnectedUnverified"
        : networkAvailability === "available" ? "filesLocationOpenVerified"
          : networkAvailability === "permission_required" ? "filesLocationPermission"
            : networkAvailability === "offline" ? "filesLocationOffline" : "filesLocationUnknown";
      return <div key={location.id} className="flex min-w-0 items-center gap-1">
        <button type="button" className="file-library-navigation-item min-w-0 flex-1"
          disabled={(!location.available && !networkLocation) || mounted.ejectingId === location.id} title={location.path} onClick={() => onOpen(location)}>
          <Icon size={16} aria-hidden="true" />
          <span className="min-w-0 truncate">{location.displayName || location.path}
            <small className="block truncate text-muted-foreground">{networkLocation ? t(networkStatusKey) : !location.available ? t(location.volume?.availability === "permission_required" ? "filesLocationPermission" : location.volume?.availability === "unknown" ? "filesLocationUnknown" : "filesDeviceDisconnected")
              : [location.volume?.readOnly ? t("filesVolumeReadOnly") : null,
                location.volume?.availableBytes != null ? `${formatBytes(location.volume.availableBytes)} ${t("filesSpaceAvailable")}` : null].filter(Boolean).join(" · ")}</small>
          </span>
        </button>
        {location.volume?.canEject && location.available ? <button type="button" className="rounded p-1.5 text-muted-foreground hover:bg-muted focus-visible:outline focus-visible:outline-2 disabled:opacity-50"
          disabled={transferring || mounted.ejectingId !== null} aria-label={`${t("filesEjectVolume")} ${location.displayName}`} title={t("filesEjectVolume")}
          onClick={() => void mounted.eject(location)}><Unplug size={15} aria-hidden="true" /></button> : null}
      </div>;
    })}
  </>;
}
