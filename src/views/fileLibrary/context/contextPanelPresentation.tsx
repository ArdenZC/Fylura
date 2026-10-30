import { createContext, useContext, type ReactNode } from "react";

export type FileLibraryWorkspaceLayout = "large" | "medium" | "compact";

interface ContextPanelPresentation {
  layout: FileLibraryWorkspaceLayout;
  paneOverlay: boolean;
}

const ContextPanelPresentationContext = createContext<ContextPanelPresentation>({
  layout: "compact",
  paneOverlay: false
});

export function ContextPanelPresentationProvider({
  layout,
  paneOverlay = false,
  children
}: {
  layout: FileLibraryWorkspaceLayout;
  paneOverlay?: boolean;
  children: ReactNode;
}) {
  return (
    <ContextPanelPresentationContext.Provider value={{ layout, paneOverlay }}>
      {children}
    </ContextPanelPresentationContext.Provider>
  );
}

export function useContextPanelPresentation(): ContextPanelPresentation {
  return useContext(ContextPanelPresentationContext);
}
