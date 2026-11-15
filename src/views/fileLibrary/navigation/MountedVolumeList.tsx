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
      const offline = networkLocation && (networkAvailability === "offline" || networkAvailability === "permission_required");
      const total = location.volume?.totalBytes ?? null;
      const available = location.volume?.availableBytes ?? null;
      const usedPercent = total && total > 0 && available !== null
        ? Math.round(Math.min(100, Math.max(0, ((total - available) / total) * 100)))
        : null;
      const chip: { key: "filesVolumeReadOnly" | "filesCloudProvider" | "filesDeviceOfflineChip"; variant: string } | null = location.volume?.readOnly
        ? { key: "filesVolumeReadOnly", variant: "is-ro" }
        : location.kind === "cloud_provider"
          ? { key: "filesCloudProvider", variant: "is-cloud" }
          : offline
            ? { key: "filesDeviceOfflineChip", variant: "is-off" }
            : null;
      const detail = networkLocation
        ? t(networkStatusKey)
        : !location.available
          ? t(location.volume?.availability === "permission_required" ? "filesLocationPermission" : location.volume?.availability === "unknown" ? "filesLocationUnknown" : "filesDeviceDisconnected")
          : available !== null
            ? `${t("filesSpaceAvailable")} ${formatBytes(available)}${total !== null ? ` / ${formatBytes(total)}` : ""}`
            : t("filesSpaceUnknown");
      return <div key={location.id} className="file-library-device-row" data-device-row data-device-id={location.id} data-device-offline={offline ? "true" : undefined}>
        <Icon size={16} aria-hidden="true" className="file-library-device-icon" />
        <button type="button" className="file-library-device-open"
          disabled={(!location.available && !networkLocation) || mounted.ejectingId === location.id} title={location.path} onClick={() => onOpen(location)}>
          <span className="file-library-device-name">
            <b>{location.displayName || location.path}</b>
            {chip ? <span className={`file-library-device-chip ${chip.variant}`}>{t(chip.key)}</span> : null}
          </span>
          <small className="file-library-device-detail">{detail}</small>
          {usedPercent !== null ? <span className="file-library-device-bar" aria-hidden="true"><i style={{ width: `${usedPercent}%` }} /></span> : null}
        </button>
        {location.volume?.canEject && location.available ? <button type="button" className="file-library-device-eject"
          disabled={transferring || mounted.ejectingId !== null} aria-label={`${t("filesEjectVolume")} ${location.displayName}`} title={t("filesEjectVolume")}
          onClick={() => void mounted.eject(location)}><Unplug size={14} aria-hidden="true" /></button> : null}
      </div>;
    })}
  </>;
}
