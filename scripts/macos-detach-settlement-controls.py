#!/usr/bin/env python3
"""Temporary isolated macOS proof controls; self-check runs no Cargo/native code."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import textwrap

FIXTURE = Path("src/BatCave.App/src-tauri/tests/macos_dmg_destination_gate_spike.rs")
DETACH = "macos::detach_supervision_failure_retains_source_until_process_settles"
DRIFT = "macos::bundle_identity_drift_fails_the_consumed_destination_hook"
TARGET = "aarch64-apple-darwin"
POST_DETACH = (
    "            // Detach is another owned process: its supervision can retain a child\n"
    "            // even when all earlier destination commands had already settled.\n"
    "            let process_settled = self.unsettled_process.is_none();\n"
)
MUTATIONS = {
    "bypass-post-detach-priority": (
        POST_DETACH + "            if !process_settled {\n",
        POST_DETACH + "            if false && !process_settled {\n",
        "RetainedProcessUnsettled",
    ),
    "report-stale-settlement": (
        "            ProbeOutcome {\n"
        "                disposition,\n"
        "                primary_boundary,\n"
        "                retained_boundary,\n"
        "                process_started,\n"
        "                process_settled,\n",
        "            ProbeOutcome {\n"
        "                disposition,\n"
        "                primary_boundary,\n"
        "                retained_boundary,\n"
        "                process_started,\n"
        "                process_settled: true,\n",
        "detach process remains unsettled",
    ),
    "release-source-before-settlement": (
        POST_DETACH,
        "            self.source.take();\n" + POST_DETACH,
        "unsettled detach retains the image source",
    ),
}


def mutation(source, name):
    before, after, _ = MUTATIONS[name]
    if source.count(before) != 1:
        raise RuntimeError(f"{name}: intended mutation must match exactly once")
    patched = source.replace(before, after, 1)
    # Every intended assertion occurs while an actual child may be retained.
    # Preserve the original panic after existing bounded native recovery proves
    # no process/mount/root was abandoned by this control-only failing case.
    header = f"    fn {DETACH.split('::')[-1]}() {{\n"
    start = patched.index(header) + len(header)
    next_test = patched.index("    #[test]\n", start)
    end = patched.rfind("    }\n", start, next_test)
    if end < start:
        raise RuntimeError("detach control test body boundary not found")
    body = patched[start:end]
    wrapper = (
        "        let control_outcome = std::panic::catch_unwind(|| {\n"
        + textwrap.indent(body, "    ")
        + "        });\n"
        + '        recover_retained().expect("CONTROL_TEARDOWN_FAILED");\n'
        + '        println!("CONTROL_TEARDOWN_SETTLED");\n'
        + "        if let Err(payload) = control_outcome {\n"
        + "            std::panic::resume_unwind(payload);\n"
        + "        }\n"
    )
    return patched[:start] + wrapper + patched[end:]


def classify(output, returncode, exact_test, expected_failure):
    output = re.sub(r"\x1b\[[0-9;]*m", "", output)
    if expected_failure is None:
        count = 14 if exact_test is None else 1
        passed = (
            returncode == 0
            and re.search(rf"test result: ok\. {count} passed; 0 failed;", output)
            and (exact_test is None or f"test {exact_test} ... ok" in output)
        )
        return bool(passed), "original suite passed" if passed else "native/setup failure"
    intended = (
        returncode == 101
        and f"test {exact_test} ... FAILED" in output
        and f"---- {exact_test} stdout ----" in output
        and "test result: FAILED. 0 passed; 1 failed;" in output
        and "panicked at" in output
        and "owned_fixture_supervision:stage=detach;after_spawn_unsettled" in output
        and expected_failure in output
        and "CONTROL_TEARDOWN_SETTLED" in output
        and "CONTROL_TEARDOWN_FAILED" not in output
    )
    if expected_failure == "RetainedProcessUnsettled":
        intended = intended and re.search(r"right:\s+RetainedProcessUnsettled\b", output)
    return bool(intended), "intended guard failed with settled teardown" if intended else "wrong failure/setup/teardown failure"


def self_check(source):
    for marker in (
        "let process_settled = self.unsettled_process.is_none();",
        '"unsettled detach retains the image source"',
        '"failed settlement retains the image source"',
        '"detach process remains unsettled"',
        "Fault::DetachSupervisionSettlementFailure",
        "assert!(!outcome.mount_residue);",
    ):
        if marker not in source:
            raise RuntimeError(f"reviewed fixture marker missing: {marker}")
    for name, definition in MUTATIONS.items():
        patched = mutation(source, name)
        if patched == source or patched.count("CONTROL_TEARDOWN_SETTLED") != 1:
            raise RuntimeError(f"{name}: invalid mutation/teardown")
        expected = definition[2]
        synthetic = (
            f"test {DETACH} ... FAILED\n---- {DETACH} stdout ----\n"
            "owned_fixture_supervision:stage=detach;after_spawn_unsettled;pid=1;process_group=1\n"
            f"thread 'control' panicked at fixture:1:1:\n{expected}\n"
            + ("left: RetainedCleanupFailed\nright: RetainedProcessUnsettled\n" if expected == "RetainedProcessUnsettled" else "")
            + "CONTROL_TEARDOWN_SETTLED\ntest result: FAILED. 0 passed; 1 failed;\n"
        )
        if not classify(synthetic, 101, DETACH, expected)[0]:
            raise RuntimeError(f"{name}: classifier rejects intended receipt")
        for bad in (
            synthetic.replace("CONTROL_TEARDOWN_SETTLED", "CONTROL_TEARDOWN_FAILED"),
            synthetic.replace(expected, "acquire detach authority failed"),
            synthetic.replace(f"---- {DETACH} stdout ----", ""),
            synthetic.replace("owned_fixture_supervision:stage=detach;after_spawn_unsettled", ""),
        ):
            if classify(bad, 101, DETACH, expected)[0]:
                raise RuntimeError(f"{name}: classifier accepted a setup/teardown/wrong-stage failure")
        if classify(synthetic, 0, DETACH, expected)[0]:
            raise RuntimeError(f"{name}: classifier accepted successful Cargo status")
        # Parse each isolated control without compiling or executing any native code.
        with tempfile.TemporaryDirectory(prefix="batcave-detach-control-parse-") as directory:
            fixture = Path(directory) / "control.rs"
            fixture.write_text(patched, encoding="utf-8", newline="\n")
            subprocess.run(["rustfmt", "--edition", "2021", str(fixture)], check=True)
    return hashlib.sha256(source.encode()).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path.cwd())
    parser.add_argument("--self-check", action="store_true")
    parser.add_argument("--expected-head")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    repo = args.repo.resolve(strict=True)
    source = (repo / FIXTURE).read_text(encoding="utf-8")
    fixture_sha256 = self_check(source)
    if args.self_check:
        print(json.dumps({"self_check": "passed", "fixture_sha256": fixture_sha256,
                          "controls": list(MUTATIONS), "control_only_teardown": True}))
        return
    if sys.platform != "darwin":
        raise RuntimeError("actual execution requires hosted macOS; self-check is static only")
    if not args.expected_head or args.output is None:
        raise RuntimeError("hosted proof requires expected immutable head and output directory")
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
    if head != args.expected_head:
        raise RuntimeError("checkout differs from expected hosted source")
    tracked = subprocess.check_output(["git", "show", f"HEAD:{FIXTURE.as_posix()}"], cwd=repo, text=True)
    if tracked != source:
        raise RuntimeError("fixture differs from immutable tracked source")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    receipt = {"source_head": head, "fixture_sha256": fixture_sha256, "target": TARGET,
               "cases": [], "all_required_controls_passed": False}
    receipt_path = output / "receipt.json"
    env = os.environ.copy()
    env.update({"CARGO_TERM_COLOR": "never", "RUSTC_WRAPPER": "",
                "RUSTC_WORKSPACE_WRAPPER": "",
                "CARGO_TARGET_DIR": str(repo / "src/BatCave.App/src-tauri/target")})
    cases = [
        ("baseline-detach", DETACH, None),
        ("baseline-drift", DRIFT, None),
        *[(name, DETACH, definition[2]) for name, definition in MUTATIONS.items()],
        ("restored-detach", DETACH, None),
        ("restored-drift", DRIFT, None),
        ("restored-native-integration", None, None),
    ]
    try:
        with tempfile.TemporaryDirectory(prefix="batcave-macos-detach-controls-") as directory:
            scratch = Path(directory)
            archive_path = scratch / "source.tar"
            with archive_path.open("wb") as archive_file:
                subprocess.run(["git", "archive", "--format=tar", "HEAD"], cwd=repo,
                               stdout=archive_file, check=True)
            original = scratch / "original"
            original.mkdir(mode=0o700)
            with tarfile.open(archive_path) as archive:
                archive.extractall(original, filter="data")
            for name, exact_test, expected in cases:
                isolated = scratch / name
                shutil.copytree(original, isolated)
                patched = mutation(source, name) if name in MUTATIONS else source
                (isolated / FIXTURE).write_text(patched, encoding="utf-8", newline="\n")
                command = ["cargo", "test", "--locked", "--manifest-path",
                           str(isolated / "src/BatCave.App/src-tauri/Cargo.toml"), "--target", TARGET,
                           "--test", "macos_dmg_destination_gate_spike"]
                if exact_test is not None:
                    command += [exact_test, "--", "--exact"]
                print(f"Running isolated case: {name}", flush=True)
                result = subprocess.run(command, cwd=isolated, env=env, text=True,
                                        stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
                log = output / f"{name}.log"
                log.write_text(result.stdout, encoding="utf-8", newline="\n")
                accepted, disposition = classify(result.stdout, result.returncode, exact_test, expected)
                receipt["cases"].append({"name": name, "test": exact_test,
                    "mutation": name if name in MUTATIONS else None,
                    "control_only_teardown": name in MUTATIONS,
                    "patched_fixture_sha256": hashlib.sha256(patched.encode()).hexdigest(),
                    "exit_code": result.returncode, "accepted": accepted,
                    "disposition": disposition, "log": log.name})
                receipt_path.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
                print(f"{name}: cargo={result.returncode}; {disposition}", flush=True)
                if name.startswith("baseline-") and not accepted:
                    raise RuntimeError("native baseline failed; controls cannot establish proof")
        receipt["all_required_controls_passed"] = all(case["accepted"] for case in receipt["cases"])
        if not receipt["all_required_controls_passed"]:
            raise RuntimeError("control, restoration or original native suite failed")
    finally:
        receipt_path.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
