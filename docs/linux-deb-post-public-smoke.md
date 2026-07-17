# Linux deb post-public smoke

Published releases get separate fresh, protected `ubuntu-22.04` deb and AppImage jobs after the release has been made public. This document covers the deb job. The fixed `scripts/linux-deb-post-public-smoke.mjs` and `scripts/linux-appimage-post-public-smoke.mjs` entrypoints share `scripts/linux-post-public-smoke.mjs`, but each selects one closed native capture and retains its own sanitized observation and output filename.

## Identity and byte boundary

The finalize job retains its exact pre-publication candidate JSON as a one-day workflow artifact. The post-public job downloads that fixed artifact name and invokes `scripts/linux-deb-post-public-smoke.mjs` with only the workflow-owned release tag and source SHA. Its shared driver reads the candidate from one fixed repository-relative location. Neither entrypoint accepts a caller-selected profile, artifact path, command, environment, status, callback, or evidence payload.

The new job then:

1. anonymously reads the public release API;
2. compares the release tag, source SHA, channel, immutable state, complete asset set, sizes, digests, and public URLs to the independent candidate inventory;
3. anonymously downloads every release asset;
4. verifies every downloaded digest and the complete `SHA256SUMS.txt` subject set; and
5. runs GitHub release and per-subject attestation verification against protected `main`, the exact source SHA, the pinned release workflow, and GitHub-hosted runners.

Only the unforgeable in-process verifier receipt can select the exact deb. The capture copies that file into a private root and rejects size or digest drift before package metadata inspection or privileged execution.

## Privileged operation boundary

Install and purge use a new random fixed-prefix transient service. The only root command path is:

```text
sudo -n systemd-run --wait --pipe --collect --service-type=exec ... /usr/bin/dpkg --install|--purge
```

The fixed service properties require `KillMode=control-group`, `SendSIGKILL=yes`, a ten-second stop timeout, a 120-second runtime maximum, 256 tasks at most, read-only control groups, and no delegation. Output and client runtime are bounded. HUP, INT, TERM, success, command failure, timeout, and output overflow all converge on a fixed `systemctl stop` plus repeated inactive-or-collected checks after the `systemd-run` client has settled.

Before package mutation, a fixed hostile service starts both an ordinary background sleeper and a `setsid` sleeper. The job requires the service to settle and both PIDs to disappear. Fixed systemd-owned `apt-get update` and `apt-get install` units then establish the exact Ubuntu runtime dependency set: `libgtk-3-0`, `libwebkit2gtk-4.1-0`, `libayatana-appindicator3-1`, `librsvg2-2`, and `libxdo3`. Successful prerequisite, install, and purge units all return process-local branded settlement receipts; reconstructed receipts are rejected. Prerequisites remain host setup and are not folded into BatCave package evidence.

## Native observations and cleanup

The installed package must own executable regular GUI and CLI files. The job records every existing non-directory path from the package-owned inventory, then runs:

- an exact source/version/install-kind packaged identity check;
- settings initialization and restart preservation;
- corrupt-settings degradation with visible persistence failure and preserved corrupt bytes; and
- a fixed two-tick strict packaged CLI benchmark that requires advancing core-runtime telemetry.

Purge is unconditional after any install attempt. Inventory acquisition errors cannot skip it. Cleanup distinguishes a truly absent dpkg record from a failed query, treats dangling links as residue, requires the GUI, CLI, and observed package-owned files to be gone, preserves the documented current-user state root, and checks an outside sentinel.

The script writes and prints sanitized JSON only after public verification, native checks, root-unit settlement, purge, residue checks, and workspace cleanup complete. The workflow retains that one fixed JSON file for 30 days. The result is `linux_deb_post_public_observation`, not `release_evidence`: `release_evidence_eligible` remains false and the current-user `native_candidate` packet is never promoted.
