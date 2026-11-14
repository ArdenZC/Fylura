import { useCallback, useEffect, useRef, useState } from "react";
import { SETTINGS_SECTION_EVENT } from "../../../components/spotlight/commandRegistry";
import { scrollSettingsSectionIntoView } from "../components/SettingsPrimitives";
import {
  isProgressiveSettingsSectionId,
  settingsNavigationSectionId,
  settingsSectionRequestTarget
} from "../settingsSectionModel";

export { SETTINGS_SECTION_IDS } from "../settingsSectionModel";

type SettingsFocusOptions = { focusContent?: boolean };

/**
 * v3.40 §8.2: Settings renders one section at a time. Navigation is explicit
 * state instead of a scroll-spy, but deep links, focus placement, disclosure
 * reveal and the single scroll owner stay unchanged.
 */
export function useSettingsNavigationController() {
  const [activeSettingsSection, setActiveSettingsSection] = useState("settings-general");
  const settingsScrollRef = useRef<HTMLDivElement | null>(null);

  const focusSettingsSection = useCallback((sectionId: string, options: SettingsFocusOptions = {}) => {
    const targetId = settingsSectionRequestTarget(sectionId);
    const navigationId = settingsNavigationSectionId(targetId);
    setActiveSettingsSection(navigationId);
    window.requestAnimationFrame(() => {
      const container = settingsScrollRef.current;
      if (!container) return;
      container.scrollTop = 0;
      scrollSettingsSectionIntoView(container, targetId, {
        ...options,
        revealContent: isProgressiveSettingsSectionId(targetId) || targetId !== navigationId
      });
    });
  }, []);

  useEffect(() => {
    function handleSectionRequest(event: Event) {
      const sectionId = (event as CustomEvent<string>).detail;
      if (sectionId) focusSettingsSection(sectionId);
    }

    window.addEventListener(SETTINGS_SECTION_EVENT, handleSectionRequest);
    let hasPendingSection = false;
    try {
      const pendingSection = window.sessionStorage.getItem(SETTINGS_SECTION_EVENT);
      if (pendingSection) {
        hasPendingSection = true;
        window.sessionStorage.removeItem(SETTINGS_SECTION_EVENT);
        focusSettingsSection(pendingSection);
      }
    } catch {
      // In-memory events still work when storage is unavailable.
    }
    if (!hasPendingSection) {
      const container = settingsScrollRef.current;
      if (container) container.scrollTop = 0;
      setActiveSettingsSection("settings-general");
    }
    return () => window.removeEventListener(SETTINGS_SECTION_EVENT, handleSectionRequest);
  }, [focusSettingsSection]);

  return { activeSettingsSection, focusSettingsSection, settingsScrollRef };
}
