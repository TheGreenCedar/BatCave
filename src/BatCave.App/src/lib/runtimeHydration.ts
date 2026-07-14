import type { RuntimeSnapshot } from "./types";

export interface AutomaticHydrationRequests {
  persistUiPreferences: boolean;
  syncRuntimeQuery: boolean;
}

export interface AutomaticHydrationMutations {
  persistUiPreferences: () => void;
  syncRuntimeQuery: () => void;
}

export function dispatchAutomaticRuntimeHydration(
  snapshot: RuntimeSnapshot,
  requests: AutomaticHydrationRequests,
  mutations: AutomaticHydrationMutations,
): void {
  const settingsPersistence = snapshot.persistence?.components.find(
    (component) => component.owner === "current_user" && component.kind === "settings",
  );
  if (settingsPersistence?.state !== "healthy" || settingsPersistence.active_failure !== null) {
    return;
  }
  if (requests.persistUiPreferences) mutations.persistUiPreferences();
  if (requests.syncRuntimeQuery) mutations.syncRuntimeQuery();
}
