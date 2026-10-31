import { create } from "zustand";
import { useEffect, useId, useRef } from "react";
import type { EntryRef, BrowseEntryRef, BrowsePathRef } from "../../../types/fileWorkspace";
import type { LibrarySelectionV1, OperationLog } from "../../../types/domain";
import type { TransferTicket, TransferAction } from "../../../api/fileManagementApi";
import { usePane } from "../workbench/PaneContext";
import type { BrowseSourceOwner } from "../browse/browseSourceOwner";

export interface ManagementSurface {
  browseSource?: BrowseSourceOwner;
  signature: string;
  count: number | null;
  hasDirectorySelection?: boolean;
  selection?: LibrarySelectionV1;
  refs?: EntryRef[];
  browse?: { sessionId: string; pathRef: BrowsePathRef };
  routingHint?: string;
  visible?: boolean;
  refresh: () => Promise<void>;
  clear: () => void;
  selectAll?: () => void;
  selectionScope?: "loaded" | "all";
}
export interface PendingPaneTransfer {
  paneId: string; sourcePaneId?: string; sourceSignature?: string;
  destinationSignature?: string;
  tickets: TransferTicket[]; action: TransferAction; destination: string;
  destinationEntry?: BrowseEntryRef; external?: boolean;
}
interface ManagementState {
  owner: string | null;
  surface: (() => ManagementSurface) | null;
  surfaces: Record<string, { owner: string; read: () => ManagementSurface }>;
  activePaneId: string | null;
  activatePane: (paneId: string) => void;
  pendingTransfer: PendingPaneTransfer | null;
  basket: TransferTicket[];
  clipboard: { action: TransferAction; tickets: TransferTicket[] } | null;
  results: OperationLog[];
  statuses: Record<string, string>;
  basketOpen: boolean;
  transferOpen: boolean;
  transferActive: boolean;
  transferRunning: boolean;
  activeTransferId: string | null;
  transferTicketIds: string[];
  externalDropAction: TransferAction;
  pendingAction: "copy" | "cut" | "paste" | "move" | "basket" | "rename" | "trash" | "create_directory" | "copy_other" | "move_other" | null;
  requestAction: (action: ManagementState["pendingAction"]) => void;
}
/** Shared tickets, clipboard and outcomes; every source projection has an explicit pane owner. */
export const useFileManagementStore = create<ManagementState>((set, get) => ({
  owner: null, surface: null, surfaces: {}, activePaneId: null, pendingTransfer: null, transferOpen: false,
  transferActive: false, transferRunning: false, activeTransferId: null, externalDropAction: "copy",
  basket: [], clipboard: null, results: [], statuses: {}, basketOpen: false, transferTicketIds: [], pendingAction: null,
  activatePane: (activePaneId) => {
    const entry = get().surfaces[activePaneId];
    set({ activePaneId, owner: entry?.owner ?? null, surface: entry?.read ?? null, basketOpen: false });
  },
  requestAction: (pendingAction) => set({ pendingAction })
}));
export function useFileManagementSurface(surface: ManagementSurface) {
  const owner = useId(); const pane = usePane(); const paneId = pane?.id;
  const latest = useRef(surface); latest.current = { ...surface, routingHint: pane?.routingHint, visible: pane?.visible ?? true };
  useEffect(() => {
    const read = () => latest.current;
    if (!paneId) {
      useFileManagementStore.setState({ owner, surface: read });
      return () => { if (useFileManagementStore.getState().owner === owner) useFileManagementStore.setState({ owner: null, surface: null }); };
    }
    useFileManagementStore.setState((state) => ({ surfaces: { ...state.surfaces, [paneId]: { owner, read } },
      ...(state.activePaneId === paneId || pane?.focused ? { owner, surface: read, activePaneId: paneId } : {}) }));
    return () => {
      useFileManagementStore.setState((state) => {
        if (state.surfaces[paneId]?.owner !== owner) return {};
        const surfaces = { ...state.surfaces }; delete surfaces[paneId];
        return { surfaces, ...(state.owner === owner ? { owner: null, surface: null } : {}) };
      });
    };
  }, [owner, paneId]);
  // Selection updates notify projections without briefly unregistering the live source.
  useEffect(() => {
    if (!paneId) {
      if (useFileManagementStore.getState().owner === owner) useFileManagementStore.setState({ surface: () => latest.current });
      return;
    }
    useFileManagementStore.setState((state) => {
      const entry = state.surfaces[paneId]; if (entry?.owner !== owner) return {};
      return { surfaces: { ...state.surfaces }, ...(state.activePaneId === paneId ? { owner, surface: entry.read } : {}) };
    });
  }, [owner, paneId, surface.signature, pane?.routingHint, pane?.focused]);
}
