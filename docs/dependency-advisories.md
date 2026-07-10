# Dependency advisory policy

BatCave treats Rust vulnerabilities and informational maintenance warnings as separate gates. `cargo audit` must report zero vulnerabilities. Informational warnings are accepted only when their exact advisory ID, kind, package, and version appear in `.github/security/cargo-audit-baseline.json` with an owner, runtime reachability, upstream blocker, and unexpired review date.

The baseline is owned by the BatCave maintainers and expires on 2026-10-10. The scheduled and manual `Dependency advisory audit` workflow rejects new warnings, changed package versions, removed-but-still-listed warnings, incomplete metadata, and expired reviews. Update the baseline only after reviewing the dependency path and recording why BatCave cannot remove the warning directly.

The 2026-07-10 review upgraded Tauri from 2.10 to 2.11 and reduced the inherited warning set from 20 to 17. It removed warnings through obsolete `fxhash`, `rand 0.7`, and `anyhow 1.0.102` paths. The retained set is:

- GTK3 and GLib warnings reachable only in the Linux Tauri/WebKitGTK runtime. Tauri 2 still uses GTK3, so removal requires an upstream-supported GTK4 runtime migration. BatCave does not directly call the affected `glib::VariantStrIter` API.
- `proc-macro-error`, used at Linux build time by the inherited GTK3 macros.
- `unic-*` warnings inherited through `tauri-utils` and `urlpattern`; BatCave has no direct dependency or supported replacement seam.

Re-review on every Tauri upgrade and no later than the expiry date. Keep Linux validation plus deb/AppImage packaging green after dependency changes.
