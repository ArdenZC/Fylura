import { createContext, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  FileLibraryExperienceController,
  type FileLibraryExperienceState
} from "./fileLibraryExperience";

interface FileLibraryExperienceContextValue {
  controller: FileLibraryExperienceController;
  state: FileLibraryExperienceState;
}

const FileLibraryExperienceContext = createContext<FileLibraryExperienceContextValue | null>(null);

export function FileLibraryExperienceProvider({
  active,
  children,
  controller: suppliedController,
  disposeOnUnmount = true
}: {
  active: boolean;
  children: ReactNode;
  controller?: FileLibraryExperienceController;
  disposeOnUnmount?: boolean;
}) {
  const [controller] = useState(
    () => suppliedController ?? new FileLibraryExperienceController()
  );
  const [state, setState] = useState<FileLibraryExperienceState>(() => controller.getState());
  const pendingDisposeRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const activeRef = useRef(active);
  const lifecycleVersionRef = useRef(0);

  useEffect(() => {
    const unsubscribe = controller.subscribe(setState);
    return () => {
      unsubscribe();
    };
  }, [controller]);

  useEffect(() => {
    activeRef.current = active;
    const lifecycleVersion = lifecycleVersionRef.current + 1;
    lifecycleVersionRef.current = lifecycleVersion;
    if (active) {
      void controller.resume().then(() => {
        // If the route changed again while cleanup was finishing, honor the
        // newer inactive state after the pending resume completes.
        if (lifecycleVersionRef.current !== lifecycleVersion && !activeRef.current) {
          void controller.suspend();
        }
      });
    } else {
      void controller.suspend();
    }
  }, [active, controller]);

  useEffect(() => {
    if (pendingDisposeRef.current !== null) {
      clearTimeout(pendingDisposeRef.current);
      pendingDisposeRef.current = null;
    }
    return () => {
      if (!disposeOnUnmount) return;
      // React StrictMode replays effects during development. Deferring final
      // disposal by one task keeps that replay from destroying the AppShell-
      // lifetime owner while still disposing it after the real owner unmounts.
      pendingDisposeRef.current = setTimeout(() => {
        pendingDisposeRef.current = null;
        void controller.dispose();
      }, 0);
    };
  }, [controller, disposeOnUnmount]);

  const value = useMemo(() => ({ controller, state }), [controller, state]);
  return <FileLibraryExperienceContext.Provider value={value}>{children}</FileLibraryExperienceContext.Provider>;
}

export function useFileLibraryExperience() {
  const value = useContext(FileLibraryExperienceContext);
  if (!value) throw new Error("useFileLibraryExperience must be used within FileLibraryExperienceProvider.");
  return value;
}
