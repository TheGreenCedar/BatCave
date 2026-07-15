# macOS updater post-public staging observation

The protected release workflow runs one fresh `macos-15` job after publication for the universal macOS updater archive. The job invokes the private Rust release verifier with only the release tag and the closed `macos-updater` profile. Rust independently reads the immutable public release, verifies its complete inventory, checksums, source-bound build and release attestations, and retains the exact selected archive bytes before native dispatch.

The macOS updater observer then verifies the Tauri updater signature with the public key compiled into the exact release source. It consumes the retained bytes as one bounded gzip/tar stream, rejects links, special entries, traversal, nested apps, duplicate paths, macOS case or normalization collisions, extra gzip members, and size or accounting overflow, and preflights every entry before creating a private mode-`0700` root. A second pass materializes only the recorded directories and regular files with no-follow, create-new semantics, rechecks every header and file digest, compares the complete staged tree with the preflight inventory, and removes the private root before success.

The retained JSON is `macos_updater_post_public_observation`, not release evidence. It records only the public release and asset identity, the bounded checks that passed, and explicit limitations. The updater archive is staging-only: this job does not install or launch the app, recheck Developer ID/notarization/stapling at the staged destination, exercise settings, telemetry, degradation, or A-to-B updating, or mint a native receipt or #98 packet. It does not touch the DMG path; the rejected DiskImages descriptor/path fallback remains rejected.

Run the focused contracts on macOS with:

```sh
cargo test --locked --manifest-path src/BatCave.App/src-tauri/Cargo.toml \
  --bin batcave-install-smoke --features private-release-verifier
node --test scripts/validate-macos-updater-post-public-observation.test.mjs \
  scripts/verify-release-controls.test.mjs
```
