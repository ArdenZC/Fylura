import { createContext, useContext, useCallback, type RefObject } from "react";
import type { LibraryScope } from "../../../types/domain";
import { defaultFileLibraryProjection } from "../../../store/useFileLibraryV2Store";
import type { PreviewExperienceController } from "../preview/previewExperienceController";

export interface PaneContextValue {
  id: string;
  visible: boolean;
  focused: boolean;
  rootRef: RefObject<HTMLDivElement | null>;
  projection: typeof defaultFileLibraryProjection;
  scope: LibraryScope;
  setScope: (scope: LibraryScope) => void;
  routingHint: string;
  updateLocation: (path: string) => void;
  activate: () => void;
  peerId?: string;
  activatePeer: () => void;
  registerPreview: (controller: PreviewExperienceController | null) => void;
  beforeMutation: () => Promise<void>;
  afterMutation: (directories: string[]) => Promise<void>;
}
export const PaneContext = createContext<PaneContextValue | null>(null);
export function usePane() {
  return useContext(PaneContext);
}
export function usePaneProjection() {
  return usePane()?.projection ?? defaultFileLibraryProjection;
}
export function usePaneQuerySelector() {
  const pane = usePane();
  return useCallback(
    (selector: string) =>
      pane?.rootRef.current?.querySelector<HTMLElement>(selector) ??
      (!pane ? document.querySelector<HTMLElement>(selector) : null),
    [pane?.rootRef],
  );
}
