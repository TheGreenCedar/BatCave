import type { InstallableUpdateResource } from "./updateLifecycle.ts";
import {
  checkAfterClosingUpdate,
  downloadInstallAndClose,
  UpdateResourceCleanupError,
} from "./updateLifecycle.ts";

export type StableUpdateStatus =
  | "idle"
  | "checking"
  | "available"
  | "current"
  | "installing"
  | "restart_required"
  | "error";

export interface StableUpdateState {
  status: StableUpdateStatus;
  message: string;
}

type StateListener = (state: StableUpdateState) => void;

export interface VersionedInstallableUpdateResource extends InstallableUpdateResource {
  version: string;
}

export class StableUpdateController<T extends VersionedInstallableUpdateResource> {
  private pending: T | null = null;
  private readonly checkForUpdate: () => Promise<T | null>;
  private current: StableUpdateState = {
    status: "idle",
    message: "Checks only when you ask.",
  };

  constructor(checkForUpdate: () => Promise<T | null>) {
    this.checkForUpdate = checkForUpdate;
  }

  state(): StableUpdateState {
    return { ...this.current };
  }

  async check(installKind: string, onChange: StateListener): Promise<void> {
    if (this.current.status === "restart_required") return;
    if (installKind === "deb") {
      this.publish(
        "current",
        "Debian packages update through your package manager or a downloaded .deb release.",
        onChange,
      );
      return;
    }

    this.publish("checking", "Checking the stable release channel…", onChange);
    const previous = this.pending;
    try {
      this.pending = await checkAfterClosingUpdate(previous, this.checkForUpdate);
      if (this.pending) {
        this.publish("available", `Version ${this.pending.version} is available.`, onChange);
      } else {
        this.publish("current", "BatCave is up to date.", onChange);
      }
    } catch {
      this.publish(
        "error",
        "Unable to check for updates. Monitoring remains available offline.",
        onChange,
      );
    }
  }

  async install(onChange: StateListener): Promise<void> {
    const update = this.pending;
    if (!update) return;

    this.publish("installing", "Downloading, verifying, and installing the update…", onChange);
    try {
      await downloadInstallAndClose(update);
      this.pending = null;
      this.publish(
        "restart_required",
        `Version ${update.version} is installed. Close and reopen BatCave to use it.`,
        onChange,
      );
    } catch (error) {
      let message: string;
      if (error instanceof UpdateResourceCleanupError) {
        message = error.operationError
          ? "Update failed and its local selection could not be released. Retry will clean it up before checking again."
          : "Update finished, but its local selection could not be released. Retry will clean it up before checking again.";
      } else {
        this.pending = null;
        message =
          "Update verification or installation did not complete. Monitoring remains available.";
      }
      this.publish("error", message, onChange);
    }
  }

  private publish(status: StableUpdateStatus, message: string, onChange: StateListener): void {
    this.current = { status, message };
    onChange(this.state());
  }
}
