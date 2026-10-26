import { create } from "zustand";
import { useEffect, useId, useRef } from "react";
import type { EntryRef, BrowsePathRef } from "../../../types/fileWorkspace";
import type { LibrarySelectionV1, OperationLog } from "../../../types/domain";
import type { TransferTicket, TransferAction } from "../../../api/fileManagementApi";
export interface ManagementSurface {
  signature: string;
  count: number | null;
  selection?: LibrarySelectionV1;
  refs?: EntryRef[];
  browse?: { sessionId: string; pathRef: BrowsePathRef };
  refresh: () => Promise<void>;
  clear: () => void;
  selectAll?: () => void;
  selectionScope?: "loaded" | "all";
}
interface ManagementState {
  owner: string | null;
  surface: (() => ManagementSurface) | null;
  basket: TransferTicket[];
  clipboard: { action: TransferAction; tickets: TransferTicket[] } | null;
  results: OperationLog[];
  statuses: Record<string, string>;
  basketOpen: boolean;
  pendingAction: "copy" | "cut" | "paste" | "move" | "basket" | null;
  requestAction: (action: ManagementState["pendingAction"]) => void;
}
/** Projection of backend session tickets, never durable filesystem authority. */
export const useFileManagementStore = create<ManagementState>((set) => ({
  owner: null, surface: null, basket: [], clipboard: null, results: [], statuses: {}, basketOpen: false, pendingAction: null,
  requestAction: (pendingAction) => set({ pendingAction })
}));
export function useFileManagementSurface(surface: ManagementSurface) {
  const owner = useId();
  const latest = useRef(surface); latest.current = surface;
  useEffect(() => {
    useFileManagementStore.setState({ owner, surface: () => latest.current });
    return () => { if (useFileManagementStore.getState().owner === owner) useFileManagementStore.setState({ owner: null, surface: null }); };
  }, [owner, surface.signature]);
}
