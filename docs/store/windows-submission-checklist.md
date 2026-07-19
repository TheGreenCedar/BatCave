# Microsoft Store EXE submission checklist

This prepares the existing per-machine NSIS installer for a Microsoft Store MSI/EXE listing. It does not authorize a Partner Center submission or a public release.

## Package

- Use the exact versioned GitHub Release URL produced by the Store preflight receipt. Never submit `releases/latest`, a redirecting download page, or mutable bytes.
- Select **EXE**, **x64**, and installer parameters `/S`.
- Confirm the installer is self-contained, retains the offline WebView2 runtime, requests elevation only through normal UAC, and returns `0` after a successful silent install.
- Confirm the outer installer, generated uninstaller, BatCave executables, preserved Microsoft-signed dependencies, and exact-hash BatCave-re-signed Foundry Core dependency match the signed candidate inventory.
- Install and uninstall the exact candidate silently on a clean supported Windows machine before submission. Record Apps & Features, service, launch, update, and final residue observations.

## Listing

- Product name: **BatCave Monitor**
- Publisher: **Albert Najjar**
- Category: **Utilities & tools**
- Copy, features, keywords, support, privacy, and license URLs: `windows-listing.v1.json`
- Re-read every hosted URL anonymously before submission.

## Native screenshots

Capture fresh Windows application pixels from the exact signed candidate. Do not reuse browser fixtures or macOS captures.

- Overview, Cave light, healthy machine
- Overview, one meaningful pressure or limited-data state
- Explore with a workload inspector open
- Settings showing theme family/mode and local-data controls
- One compact-width view demonstrating the responsive drawer

Use the Partner Center dimensions and file formats shown at submission time. Keep Windows chrome only when it helps prove the native surface; do not include unrelated desktop content or personal process data.

## Publication boundary

- Upload no package to Partner Center in this source lane.
- Publish no tag or GitHub Release solely to exercise this checklist.
- Resume only with the protected production release context, exact signed stable candidate, anonymous URL readback, and explicit submission authorization.
