import { useEffect, useRef } from "react";
import type { PreviewExperienceController } from "./previewExperienceController";
import type { PreviewSourceProjection } from "./previewSource";

export const SELECTED_PREVIEW_LIMIT = 200;

/** Bounded Inspector navigation. No selection, sorting or pane focus mutations. */
export function useSelectedPreview(controller: Pick<PreviewExperienceController, "open" | "setSiblingNavigation" | "observeSource">, sources: readonly PreviewSourceProjection[]) {
  const currentSources = useRef(sources);
  currentSources.current = sources;
  const signature = sources.map((source) => source.key).join("\n");
  useEffect(() => () => controller.setSiblingNavigation(null), [controller, signature]);
  return (index: number, trigger: HTMLElement) => {
    const snapshot = currentSources.current;
    const source = snapshot[index];
    if (!source) return false;
    const valid = () => currentSources.current.length === snapshot.length && currentSources.current.every((item, i) => item.key === snapshot[i].key);
    const install = (position: number) => {
      const item = snapshot[position];
      controller.setSiblingNavigation({ source: item.source, generation: item.generation, currentKey: item.key, currentIndex: position, loadedCount: snapshot.length, hasMore: false,
        move: (direction) => {
          const next = position + (direction === "next" ? 1 : -1);
          if (!valid() || !snapshot[next]) return false;
          controller.observeSource(snapshot[next]);
          install(next);
          return true;
        }
      });
    };
    const opened = controller.open(source, trigger);
    if (opened) install(index);
    return opened;
  };
}
