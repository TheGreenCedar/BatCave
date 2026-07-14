import type { RuntimeSnapshot } from "./types";

export interface AutomaticHydrationRequests {
  persistUiPreferences: boolean;
  syncRuntimeQuery: boolean;
}

export interface AutomaticHydrationMutations {
  persistUiPreferences: () => void;
  syncRuntimeQuery: () => void;
}

export interface AutomaticRuntimeFocusHydration {
  desired: RuntimeSnapshot["settings"]["query"]["focus_mode"];
  requiresSync: boolean;
  visible: RuntimeSnapshot["settings"]["query"]["focus_mode"];
}

export function planAutomaticRuntimeFocusHydration(
  snapshot: RuntimeSnapshot,
  useAttentionByDefault: boolean,
): AutomaticRuntimeFocusHydration {
  const visible = snapshot.settings.query.focus_mode;
  return {
    desired: useAttentionByDefault ? "attention" : visible,
    requiresSync: useAttentionByDefault,
    visible,
  };
}

export function dispatchAutomaticRuntimeHydration(
  snapshot: RuntimeSnapshot,
  requests: AutomaticHydrationRequests,
  mutations: AutomaticHydrationMutations,
): void {
  const settingsPersistence = snapshot.persistence?.components.find(
    (component) => component.owner === "current_user" && component.kind === "settings",
  );
  if (
    requests.persistUiPreferences &&
    settingsPersistence?.state === "healthy" &&
    settingsPersistence.active_failure === null
  ) {
    mutations.persistUiPreferences();
  }
  if (requests.syncRuntimeQuery) mutations.syncRuntimeQuery();
}
