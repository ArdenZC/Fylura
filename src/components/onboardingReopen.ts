/**
 * Voluntary re-discovery of the product intro. ADR-0016 requires the four
 * core capabilities to stay reachable from Help / About without making the
 * intro a startup gate. This module deliberately has no imports so Settings
 * can request a reopen without pulling the dialog into its module graph.
 */
export const ONBOARDING_REOPEN_EVENT = "zc-onboarding-reopen";

export function requestOnboardingReopen() {
  try {
    window.dispatchEvent(new CustomEvent(ONBOARDING_REOPEN_EVENT));
  } catch {
    // Engines without CustomEvent simply keep the Settings entry inert.
  }
}
