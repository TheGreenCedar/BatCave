# BatCave Monitor privacy

BatCave Monitor reads resource and process activity on the computer where it is running. Settings, history, cached process metadata, optional locally generated explanations, and diagnostics stay on that computer.

BatCave does not include analytics, advertising, telemetry upload, remote logging, or an account system. It contacts GitHub only when the user explicitly chooses **Check now** for an update. Optional local explanation models are never downloaded at startup or when the feature is enabled; a separate, explicit download action is required on supported Windows and Linux systems.

Removing BatCave removes the application and its machine-wide Windows collector service. User-owned settings and local history follow the retention behavior documented in [Current-user state](current-user-state.md).

Questions and privacy requests can be filed through [BatCave support](https://github.com/TheGreenCedar/BatCave/issues).
