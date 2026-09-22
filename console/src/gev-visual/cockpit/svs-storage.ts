// GEV P19 SVS — localStorage round-trip for the svsEnabled toggle.
//
// Mirrors the P17 element-visibility persistence pattern: namespace
// `intelhub.cockpit.*`, write AFTER the store reducer runs (so the
// post-reduce value is what gets persisted).

export const SVS_ENABLED_STORAGE_KEY = "intelhub.cockpit.svsEnabled";

export function readPersistedSvsEnabled(): boolean | null {
  if (typeof localStorage === "undefined") return null;
  try {
    const raw = localStorage.getItem(SVS_ENABLED_STORAGE_KEY);
    if (raw === null) return null;
    return raw === "1";
  } catch {
    return null;
  }
}

export function persistSvsEnabled(enabled: boolean): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(SVS_ENABLED_STORAGE_KEY, enabled ? "1" : "0");
  } catch {
    // quota or disabled storage — best-effort
  }
}