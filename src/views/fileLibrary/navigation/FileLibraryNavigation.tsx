import { usePaneQuerySelector, usePane } from "../workbench/PaneContext";
import {
  Bookmark,
  CircleAlert,
  Cloud,
  Folder,
  HardDrive,
  Layers3,
  MapPin,
  Network,
  Shapes,
  Tag,
  type LucideIcon
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Translator } from "../../../types/ui";
import type { LocationDescriptor, LocationKind } from "../../../types/fileWorkspace";
import { detectBrowserPlatform } from "../../../utils/viewHelpers";
import type { FileLibraryExperienceController, FileLibraryExperienceState } from "../fileLibraryExperience";
import { isActivatableLocation, locationAvailabilityLabel } from "../browse/browseSourceOwner";
import { SideSheet } from "../../shared/ui";
import {
  useLibraryNavigationSurface,
  type LibraryNavigationEntry
} from "../library/libraryNavigationSurface";
import {
  projectLocationGroups,
  type LocationPresentationGroupId,
  type LocationPresentationPlatform
} from "./locationPresentation";
import { DirectoryTree, describeDirectoryTreeError } from "./DirectoryTree";
import { cn } from "../../../utils/tw";
import { currentWorkbenchBrowsePath, workbenchTargetKey } from "../workbench/workbenchModel";

export type FileLibraryNavigationLayout = "large" | "drawer";

const LOCATION_GROUP_ORDER: readonly LocationKind[] = ["local", "external", "network", "cloud_provider", "unknown"];

export function groupBrowseLocations(locations: readonly LocationDescriptor[]) {
  const groups = new Map<LocationKind, LocationDescriptor[]>();
  for (const kind of LOCATION_GROUP_ORDER) groups.set(kind, []);
  for (const location of locations) groups.get(location.kind)?.push(location);
  return LOCATION_GROUP_ORDER
    .map((kind) => ({ kind, locations: groups.get(kind) ?? [] }))
    .filter((group) => group.locations.length > 0);
}

export function locationIdentity(location: LocationDescriptor) {
  return location.ref.kind === "managed"
    ? `managed:${location.ref.scanRootId}`
    : `ephemeral:${location.ref.browseSessionId}:${location.ref.locationId}`;
}

export function isLocationNavigationActivatable(
  location: LocationDescriptor,
  mode: "library" | "browse"
) {
  return mode === "library"
    ? location.ref.kind === "managed"
    : isActivatableLocation(location);
}

export function FileLibraryNavigation({
  controller,
  state,
  layout,
  platform = detectBrowserPlatform(),
  t,
  onClose
}: {
  controller: FileLibraryExperienceController;
  state: FileLibraryExperienceState;
  layout: FileLibraryNavigationLayout;
  platform?: LocationPresentationPlatform;
  t: Translator;
  onClose: () => void;
}) {
  const [locationsLoading, setLocationsLoading] = useState(false);
  const [locationsFailed, setLocationsFailed] = useState(false);
  const [treeNavigationError, setTreeNavigationError] = useState<unknown | null>(null);
  const locationLoadStartedRef = useRef(false);
  const locations = state.workspace.locations;
  const librarySurface = useLibraryNavigationSurface();
  const managedLocationEntries = librarySurface?.managedLocations ?? [];
  const managedLocationByIdentity = useMemo(() => new Map(
    managedLocationEntries.map((entry) => [locationIdentity(entry.location), entry])
  ), [managedLocationEntries]);
  const visibleLocations = state.mode === "library"
    ? managedLocationEntries.map((entry) => entry.location)
    : locations;
  const locationGroups = useMemo(
    () => projectLocationGroups(visibleLocations, platform),
    [platform, visibleLocations]
  );

  useEffect(() => {
    if (locations.length > 0 || locationLoadStartedRef.current) return;
    let cancelled = false;
    locationLoadStartedRef.current = true;
    setLocationsLoading(true);
    setLocationsFailed(false);
    void controller.workspace.loadLocations()
      .then((loaded) => {
        if (cancelled) return;
        setLocationsFailed(loaded === null);
      })
      .catch(() => {
        if (!cancelled) setLocationsFailed(true);
      })
      .finally(() => {
        if (!cancelled) setLocationsLoading(false);
      });
    return () => { cancelled = true; };
  }, [controller.workspace, locations.length]);

  const query = usePaneQuerySelector();
  const pane = usePane();
  const currentTarget = state.workspace.session.currentTarget;
  const currentTargetKey = workbenchTargetKey(currentTarget);
  const browseTarget = currentTarget?.kind === "browse" ? currentTarget : null;
  const browse = state.workspace.browse;
  const treeAdmissionMatchesTarget = browseTarget?.location.kind === "ephemeral"
    && browse !== null
    && browseTarget.location.browseSessionId === browse.sessionId
    && browse.location.ref.kind === "ephemeral"
    && browse.location.ref.browseSessionId === browse.sessionId
    && browseTarget.location.locationId === browse.location.ref.locationId;
  const treeLocation = state.mode === "browse" && treeAdmissionMatchesTarget
    ? browse?.location ?? null
    : null;
  const treeCurrentPath = pane && browseTarget !== null
    ? currentWorkbenchBrowsePath(pane.routingHint, pane.pathTargetKey, browseTarget)
    : null;
  const treeRootPath = treeCurrentPath !== null
    && browse !== null
    && treeAdmissionMatchesTarget
    && browseTarget !== null
    && browseTarget.location.kind === "ephemeral"
    && browseTarget.pathRef.id === browse.rootPathRef.id
    ? treeCurrentPath
    : null;
  useEffect(() => {
    setTreeNavigationError(null);
  }, [currentTargetKey, pane?.routingHint, treeLocation?.ref]);
  const navigation = (
    <nav
      className="file-library-navigation-panel"
      aria-label={t("fileLibraryNavigationLabel")}
      data-file-library-navigation-panel="true"
      data-file-library-navigation-mode={layout}
    >
      {layout === "large" ? <header className="file-library-navigation-header">
        <div className="min-w-0">
          <h2 className="file-library-navigation-title">{t("fileLibraryNavigationTitle")}</h2>
          <p className="file-library-navigation-description">{t("fileLibraryNavigationDescription")}</p>
        </div>
      </header> : null}

      {state.mode === "library" ? <section className="file-library-navigation-section" aria-labelledby="file-library-navigation-library-heading">
        <h3 id="file-library-navigation-library-heading" className="file-library-navigation-section-title">{t("fileLibraryNavigationLibrary")}</h3>
        <div className="file-library-navigation-list">
          {librarySurface ? <>
            <LibraryNavigationButton item={librarySurface.all} icon={Layers3} />
            <LibraryNavigationDisclosure id="types" label={t("fileLibraryNavigationTypes")} icon={Shapes} items={librarySurface.types} emptyLabel={t("fileLibraryNavigationTypesEmpty")} />
            <LibraryNavigationDisclosure id="saved" label={t("fileLibraryNavigationSaved")} icon={Bookmark} items={librarySurface.savedViews} emptyLabel={t("fileLibraryNavigationSavedEmpty")} />
            <LibraryNavigationDisclosure id="tags" label={t("fileLibraryNavigationTags")} icon={Tag} items={librarySurface.tags} emptyLabel={t("fileLibraryNavigationTagsEmpty")} />
          </> : <p className="file-library-navigation-muted" data-file-library-navigation-library-state="unavailable">{t("fileLibraryNavigationLibraryUnavailable")}</p>}
        </div>
      </section> : null}

      {treeLocation && pane ? <DirectoryTree
        controller={controller}
        location={treeLocation}
        currentPath={treeCurrentPath}
        currentTargetKey={currentTargetKey}
        rootPath={treeRootPath}
        paneId={pane.id}
        platform={platform}
        navigationError={treeNavigationError}
        onNavigationError={(error) => setTreeNavigationError(error)}
        t={t}
      /> : null}
      {treeNavigationError !== null && !treeLocation ? <p className="file-library-navigation-muted flex items-center gap-1" role="status"><CircleAlert size={14} aria-hidden="true" />{describeDirectoryTreeError(treeNavigationError, t)}</p> : null}

      <section className="file-library-navigation-section" aria-labelledby={pane ? `file-library-navigation-locations-heading-${pane.id}` : "file-library-navigation-locations-heading"}>
        <div className="file-library-navigation-section-heading">
          <h3 id={pane ? `file-library-navigation-locations-heading-${pane.id}` : "file-library-navigation-locations-heading"} className="file-library-navigation-section-title">{state.mode === "library" ? t("fileLibraryNavigationLibraryLocations") : t("fileLibraryNavigationLocations")}</h3>
          <MapPin size={15} aria-hidden="true" />
        </div>
        {locationsLoading && locations.length === 0 ? <p className="file-library-navigation-muted" role="status">{t("fileLibraryNavigationLocationsLoading")}</p> : null}
        {locationsFailed && locations.length === 0 ? <p className="file-library-navigation-muted" role="status"><CircleAlert size={14} aria-hidden="true" />{t("fileLibraryNavigationLocationsFailed")}</p> : null}
        {!locationsLoading && !locationsFailed && visibleLocations.length === 0 ? <p className="file-library-navigation-muted">{state.mode === "library" ? t("fileLibraryNavigationManagedLocationsEmpty") : t("fileLibraryNavigationLocationsEmpty")}</p> : null}
        <div className="file-library-navigation-location-groups">
          {locationGroups.map((group) => (
            <div key={group.id} className="file-library-navigation-location-group" data-file-library-location-group={group.id}>
              <h4 className="file-library-navigation-location-group-title">{locationGroupLabel(group.id, t)}</h4>
              <div className="file-library-navigation-list">
                {group.locations.map((location) => {
                  const libraryEntry = state.mode === "library"
                    ? managedLocationByIdentity.get(locationIdentity(location))
                    : undefined;
                  if (state.mode === "library" && libraryEntry === undefined) return null;
                  return (
                    <LocationNavigationItem
                      key={locationIdentity(location)}
                      location={location}
                      onActivate={libraryEntry?.activate ?? (() => controller.browseLocation(location.ref))}
                      navigationId={libraryEntry?.id}
                      active={libraryEntry?.active ?? false}
                      mode={state.mode}
                      t={t}
                    />
                  );
                })}
              </div>
            </div>
          ))}
        </div>
      </section>

      <p className="file-library-navigation-footnote">{t("fileLibraryNavigationAuthorityNote")}</p>
    </nav>
  );

  if (layout === "drawer") {
    return (
      <SideSheet
        open
        title={t("fileLibraryNavigationTitle")}
        description={t("fileLibraryNavigationDescription")}
        onClose={onClose}
        closeLabel={t("fileLibraryNavigationClose")}
        side="left"
        modalId="file-library-navigation"
        restoreFocus={() => query("[data-file-library-nav-toggle]")}
      >
        {navigation}
      </SideSheet>
    );
  }
  return navigation;
}

function LibraryNavigationButton({ item, icon: Icon }: { item: LibraryNavigationEntry; icon: LucideIcon }) {
  return (
    <button
      className={cn("file-library-navigation-item", item.active && "is-active")}
      type="button"
      aria-current={item.active ? "page" : undefined}
      data-file-library-navigation-item={item.id}
      onClick={item.activate}
    >
      <Icon size={16} aria-hidden="true" />
      <span>{item.label}</span>
    </button>
  );
}

function LibraryNavigationDisclosure({
  id,
  label,
  icon: Icon,
  items,
  emptyLabel
}: {
  id: string;
  label: string;
  icon: LucideIcon;
  items: readonly LibraryNavigationEntry[];
  emptyLabel: string;
}) {
  const [open, setOpen] = useState(true);
  return (
    <section data-file-library-navigation-group={id}>
      <button
        className="file-library-navigation-group-toggle"
        type="button"
        aria-expanded={open}
        data-file-library-navigation-group-toggle={id}
        onClick={() => setOpen((value) => !value)}
      >
        <Icon size={16} aria-hidden="true" />
        <span>{label}</span>
      </button>
      {open ? items.length > 0 ? <div className="file-library-navigation-list file-library-navigation-sublist">
        {items.map((item) => <LibraryNavigationButton key={item.id} item={item} icon={Tag} />)}
      </div> : <p className="file-library-navigation-muted file-library-navigation-empty-child">{emptyLabel}</p> : null}
    </section>
  );
}

function LocationNavigationItem({
  location,
  onActivate,
  navigationId,
  active,
  mode,
  t
}: {
  location: LocationDescriptor;
  onActivate: () => void | Promise<unknown>;
  navigationId?: string;
  active?: boolean;
  mode: "library" | "browse";
  t: Translator;
}) {
  const activatable = isLocationNavigationActivatable(location, mode);
  const browseUnavailable = mode === "browse" && !activatable;
  const status = locationAvailabilityLabel(location.availability, t);
  const managedLabel = location.ref.kind === "managed"
    ? t("fileLibraryNavigationManaged")
    : t("fileLibraryNavigationUnmanaged");
  const Icon = location.kind === "cloud_provider"
    ? Cloud
    : location.kind === "network"
      ? Network
      : location.ref.kind === "managed"
        ? HardDrive
        : Folder;

  return (
    <div className={cn("file-library-navigation-location", browseUnavailable && "is-unavailable", active && "is-active")} data-file-library-location={locationIdentity(location)} data-file-library-location-managed={location.ref.kind === "managed" ? "true" : "false"}>
      <button
        className="file-library-navigation-location-button"
        type="button"
        disabled={!activatable}
        aria-label={`${location.displayName}, ${managedLabel}, ${status}`}
        aria-current={active ? "page" : undefined}
        data-file-library-navigation-item={navigationId}
        onClick={() => void onActivate()}
      >
        <Icon size={15} aria-hidden="true" />
        <span className="file-library-navigation-location-copy">
          <strong>{location.displayName}</strong>
          <small>{managedLabel} · {status}</small>
        </span>
      </button>
    </div>
  );
}

function locationGroupLabel(group: LocationPresentationGroupId, t: Translator) {
  switch (group) {
    case "this_pc": return t("fileLibraryNavigationGroupThisPc");
    case "locations": return t("fileLibraryNavigationGroupLocations");
    case "providers": return t("fileLibraryNavigationGroupProviders");
    case "network": return t("fileLibraryNavigationGroupNetwork");
    case "cloud": return t("fileLibraryNavigationGroupCloud");
    default: return t("fileLibraryNavigationGroupOther");
  }
}
