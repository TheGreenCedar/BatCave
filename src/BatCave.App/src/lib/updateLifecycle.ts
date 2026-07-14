export interface UpdateResource {
  close(): Promise<void>;
}

export interface InstallableUpdateResource extends UpdateResource {
  downloadAndInstall(): Promise<void>;
}

const invalidResourceId = /^The resource id \d+ is invalid\.$/;

export class UpdateResourceCleanupError extends AggregateError {
  readonly cleanupError: unknown;
  readonly operationError: unknown | null;

  constructor(cleanupError: unknown, operationError: unknown | null) {
    super(
      operationError === null ? [cleanupError] : [operationError, cleanupError],
      operationError === null
        ? "The Tauri update resource could not be closed."
        : "The update failed and its Tauri resource could not be closed.",
    );
    this.name = "UpdateResourceCleanupError";
    this.cleanupError = cleanupError;
    this.operationError = operationError;
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export async function closeUpdateResource(update: UpdateResource | null): Promise<void> {
  if (!update) return;

  try {
    await update.close();
  } catch (error) {
    // This exact error is tauri 2.11.5's ResourceTable::BadResourceId display.
    // It proves the handle is already absent. Other failures remain actionable.
    if (!invalidResourceId.test(errorMessage(error))) throw error;
  }
}

export async function checkAfterClosingUpdate<T extends UpdateResource>(
  previous: T | null,
  check: () => Promise<T | null>,
): Promise<T | null> {
  await closeUpdateResource(previous);
  return check();
}

export async function downloadInstallAndClose(update: InstallableUpdateResource): Promise<void> {
  let operationFailed = false;
  let operationError: unknown;
  try {
    await update.downloadAndInstall();
  } catch (error) {
    operationFailed = true;
    operationError = error;
  }

  try {
    await closeUpdateResource(update);
  } catch (cleanupError) {
    throw new UpdateResourceCleanupError(cleanupError, operationFailed ? operationError : null);
  }

  if (operationFailed) throw operationError;
}
