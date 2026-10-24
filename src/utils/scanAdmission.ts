import { readableError } from "./viewHelpers";

// These are the existing backend lease-admission refusals. Other errors,
// including failed admitted runs, remain failures rather than a busy notice.
export function isScanAdmissionBusy(error: unknown): boolean {
  return /^(?:Validation error: )?Scan root (?:already has an active run:|overlaps an active root lease:)/.test(readableError(error));
}
