export const TAURI_NSIS_UPSTREAM_BLOB_SHA: string;
export const upstreamTemplatePath: string;
export const generatedTemplatePath: string;

export function buildFyluraNsisTemplate(upstream: string): string;
export function prepareWindowsNsisTemplate(): string;
export function cleanWindowsNsisTemplate(): void;
