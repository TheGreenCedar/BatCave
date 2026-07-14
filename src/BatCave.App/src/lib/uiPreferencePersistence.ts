import type { RuntimeSnapshot, RuntimeUiPreferences } from "./types";

export interface UiPreferenceSave {
  generation: number;
  preferences: RuntimeUiPreferences;
}

export class UiPreferencePersistenceSequence {
  private generation = 0;

  begin(preferences: RuntimeUiPreferences): UiPreferenceSave {
    this.generation += 1;
    return {
      generation: this.generation,
      preferences: { ...preferences },
    };
  }

  isLatest(save: UiPreferenceSave): boolean {
    return save.generation === this.generation;
  }

  isLatestDurable(save: UiPreferenceSave, snapshot: RuntimeSnapshot): boolean {
    if (!this.isLatest(save)) return false;
    const published = snapshot.settings.ui_preferences;
    if (
      published?.theme !== save.preferences.theme ||
      published.history_point_limit !== save.preferences.history_point_limit
    ) {
      return false;
    }
    const settingsPersistence = snapshot.persistence?.components.find(
      (component) => component.owner === "current_user" && component.kind === "settings",
    );
    return (
      settingsPersistence?.state === "healthy" &&
      settingsPersistence.durability === "durable" &&
      settingsPersistence.active_failure === null
    );
  }
}
