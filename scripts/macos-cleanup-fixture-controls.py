#!/usr/bin/env python3
"""Temporary hosted macOS controls; --self-check performs no Cargo or native execution."""

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
CLEANUP = "macos::cleanup_failure_is_retained_and_bounded_retry_removes_residue"
PRIORITY = "macos::post_spawn_settlement_failure_retains_authority_until_retry_succeeds"
TARGET = "aarch64-apple-darwin"
MUTATIONS = {
    "bypass-cleanup-fault": (
        "        fn cleanup_root(&mut self) -> io::Result<()> {\n"
        "            let Some(root) = self.root.as_ref() else {\n"
        "                return Ok(());\n"
        "            };\n"
        "            if self.force_cleanup_failure {",
        "        fn cleanup_root(&mut self) -> io::Result<()> {\n"
        "            let Some(root) = self.root.as_ref() else {\n"
        "                return Ok(());\n"
        "            };\n"
        "            if false && self.force_cleanup_failure {",
        CLEANUP,
        "RetainedCleanupFailed",
    ),
    "bypass-unsettled-retention": (
        "            if !process_settled {\n"
        "                disposition = Disposition::RetainedProcessUnsettled;",
        "            if false && !process_settled {\n"
        "                disposition = Disposition::RetainedProcessUnsettled;",
        PRIORITY,
        "RetainedProcessUnsettled",
    ),
    "disable-retry-root-cleanup": (
        "            self.force_cleanup_failure = false;\n"
        "            self.source.take();\n"
        "            self.cleanup_root()\n",
        "            self.force_cleanup_failure = false;\n"
        "            self.source.take();\n"
        "            Ok(())\n",
        CLEANUP,
        "retry removes the retained cleanup root",
    ),
}


def mutation(source, name):
    before, after, _, _ = MUTATIONS[name]
    if source.count(before) != 1:
        raise RuntimeError(f"{name}: mutation must match exactly one intended fixture hunk")
    patched = source.replace(before, after, 1)
    if name == "bypass-unsettled-retention":
        # The intended panic comes before the injected first settlement retry.
        # Drop retains that real child; settle it with the existing recovery
        # policy before resuming the original assertion failure.
        header = f"    fn {PRIORITY.split('::')[-1]}() {{\n"
        start = patched.index(header) + len(header)
        next_test = patched.index("    #[test]\n", start)
        end = patched.rfind("    }\n", start, next_test)
        if end < start:
            raise RuntimeError("priority test body boundary not found")
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
        patched = patched[:start] + wrapper + patched[end:]
    return patched


def self_check(source):
    for marker in (
        'DestinationAuthority::acquire(b"inert cleanup fixture\\n", Vec::new())',
        'expect("cleanup precondition process settles")',
        'assert!(!root.exists(), "retry removes the retained cleanup root")',
        "assert!(authority.force_cleanup_failure);",
    ):
        if marker not in source:
            raise RuntimeError(f"reviewed fixture marker missing: {marker}")
    for name in MUTATIONS:
        patched = mutation(source, name)
        if patched == source:
            raise RuntimeError(f"{name}: empty mutation")
    return hashlib.sha256(source.encode()).hexdigest()


def classify(output, returncode, exact_test, expected_failure):
    output = re.sub(r"\x1b\[[0-9;]*m", "", output)
    if expected_failure is None:
        expected_passes = 13 if exact_test is None else 1
        passed = (
            returncode == 0
            and re.search(rf"test result: ok\. {expected_passes} passed; 0 failed;", output)
            and (exact_test is None or f"test {exact_test} ... ok" in output)
        )
        return bool(passed), "restored suite passed" if passed else "restored/setup failure"
    intended = (
        returncode == 101
        and f"test {exact_test} ... FAILED" in output
        and f"---- {exact_test} stdout ----" in output
        and "test result: FAILED. 0 passed; 1 failed;" in output
        and "panicked at" in output
        and expected_failure in output
    )
    if expected_failure.startswith("Retained"):
        intended = intended and re.search(rf"right:\s+{expected_failure}\b", output)
    if expected_failure == "RetainedProcessUnsettled":
        intended = intended and "CONTROL_TEARDOWN_SETTLED" in output
    intended = intended and "CONTROL_TEARDOWN_FAILED" not in output
    return bool(intended), "intended assertion failed" if intended else "wrong failure/setup failure"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path.cwd())
    parser.add_argument("--self-check", action="store_true")
    parser.add_argument("--expected-head")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    repo = args.repo.resolve(strict=True)
    source = (repo / FIXTURE).read_text()
    fixture_sha256 = self_check(source)
    if args.self_check:
        print(json.dumps({"self_check": "passed", "fixture_sha256": fixture_sha256,
                          "mutation_hunks": list(MUTATIONS)}))
        return
    if sys.platform != "darwin":
        raise RuntimeError("execution is hosted macOS only; use --self-check for static validation")
    if not args.expected_head or args.output is None:
        raise RuntimeError("hosted execution requires --expected-head and --output")
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
    if head != args.expected_head:
        raise RuntimeError("checkout differs from the selected hosted source")
    tracked_source = subprocess.check_output(["git", "show", f"HEAD:{FIXTURE.as_posix()}"],
                                             cwd=repo, text=True)
    if tracked_source != source:
        raise RuntimeError("fixture differs from immutable checkout source")
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
        ("baseline-cleanup", CLEANUP, None),
        ("baseline-priority", PRIORITY, None),
        *[(name, definition[2], definition[3]) for name, definition in MUTATIONS.items()],
        ("restored-cleanup", CLEANUP, None),
        ("restored-priority", PRIORITY, None),
        ("restored-native-integration", None, None),
    ]
    try:
        with tempfile.TemporaryDirectory(prefix="batcave-macos-cleanup-controls-") as temporary:
            scratch = Path(temporary)
            archive_path = scratch / "source.tar"
            with archive_path.open("wb") as archive_file:
                subprocess.run(["git", "archive", "--format=tar", "HEAD"], cwd=repo,
                               stdout=archive_file, check=True)
            original = scratch / "original"
            original.mkdir(mode=0o700)
            with tarfile.open(archive_path) as archive:
                archive.extractall(original, filter="data")
            for name, exact_test, expected_failure in cases:
                copy = scratch / name
                shutil.copytree(original, copy)
                patched = mutation(source, name) if name in MUTATIONS else source
                (copy / FIXTURE).write_text(patched, newline="\n")
                command = ["cargo", "test", "--locked", "--manifest-path",
                           str(copy / "src/BatCave.App/src-tauri/Cargo.toml"), "--target", TARGET,
                           "--test", "macos_dmg_destination_gate_spike"]
                if exact_test is not None:
                    command += [exact_test, "--", "--exact"]
                print(f"Running isolated case: {name}", flush=True)
                result = subprocess.run(command, cwd=copy, env=env, text=True,
                                        stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
                log_path = output / f"{name}.log"
                log_path.write_text(result.stdout, newline="\n")
                accepted, disposition = classify(result.stdout, result.returncode,
                                                  exact_test, expected_failure)
                receipt["cases"].append({"name": name, "test": exact_test,
                    "mutation": name if name in MUTATIONS else None,
                    "patched_fixture_sha256": hashlib.sha256(patched.encode()).hexdigest(),
                    "exit_code": result.returncode, "accepted": accepted,
                    "disposition": disposition, "log": log_path.name})
                receipt_path.write_text(json.dumps(receipt, indent=2) + "\n")
                print(f"{name}: cargo={result.returncode}; {disposition}", flush=True)
                if name.startswith("baseline-") and not accepted:
                    raise RuntimeError("baseline failed; mutation evidence would be invalid")
        receipt["all_required_controls_passed"] = all(case["accepted"] for case in receipt["cases"])
        if not receipt["all_required_controls_passed"]:
            raise RuntimeError("a control, restored test, or native integration suite failed")
    finally:
        receipt_path.write_text(json.dumps(receipt, indent=2) + "\n")


if __name__ == "__main__":
    main()
