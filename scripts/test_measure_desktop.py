"""Deterministic ownership and measurement gates; no process launch or native load."""
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from types import SimpleNamespace
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("measure_desktop", Path(__file__).with_name("measure-desktop.py"))
M = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = M
SPEC.loader.exec_module(M)


def header():
    return {"kind": "header", "schema_version": 1, "pid": 10, "warmup_seconds": 30,
            "measurement_seconds": 120, "started_at_unix_ms": 1000,
            "release_identity": {"app_version": "0.2.0", "source_commit_sha": None},
            "enhanced_narratives": False,
            "publication_boundary": "runtime_publication_to_renderer_second_animation_frame",
            "interaction_boundary": "trusted_dom_event_to_second_animation_frame"}


def observations():
    rows = [{"kind": "observation", "elapsed_ms": 30_000 + index * 1000, "enhanced_narratives": False,
             "observation": {"kind": "publication", "publication_seq": index + 1,
                             "age_ms": 50, "process_count": 500, "interval_ms": 1000}} for index in range(120)]
    rows.extend({"kind": "observation", "elapsed_ms": 40_000 + index * 1000, "enhanced_narratives": False,
                 "observation": {"kind": "interaction", "duration_ms": 12}} for index in range(10))
    return sorted(rows, key=lambda row: row["elapsed_ms"])


def native_samples():
    return [{"elapsed_ms": 30_000 + index * 1000,
             "processes": [{"pid": pid, "generation": pid + 100, "cpu_seconds": index * 0.04,
                            "rss_bytes": pid * 100, "footprint_bytes": pid * 80} for pid in [1, 2]]}
            for index in range(121)]


def coalition_samples():
    rows = native_samples()
    for index, row in enumerate(rows):
        row["coalition"] = {"resource_id": 3, "tasks_started": 2, "tasks_exited": 0,
                            "cpu_ticks": 24_000_000 + index * 2_880_000, "timebase_numer": 125, "timebase_denom": 3}
    return rows


class MacCoalitionTests(unittest.TestCase):
    def test_invisible_helper_cpu_survives_exit_but_rss_is_incomplete(self):
        rows = coalition_samples()
        for row in rows[60:]:
            row["coalition"].update(tasks_started=3, tasks_exited=1)
        result = M.resource_metrics(rows)
        self.assertAlmostEqual(result["mean_one_core_cpu_percent"], 12)
        self.assertTrue(result["cpu_coverage_complete"])
        self.assertFalse(result["rss_coverage_complete"])
        self.assertEqual(result["tasks_started_during_measurement"], 1)
        self.assertEqual(result["tasks_exited_during_measurement"], 1)
        self.assertTrue(result["coverage_errors"])

    def test_stable_coalition_preserves_ai_off_budget_gate(self):
        rows = coalition_samples()
        for row in rows:
            row["coalition"]["cpu_ticks"] //= 2
        resources = M.resource_metrics(rows)
        report = {"errors": [], "ai_mode": "off", "platform": "darwin", "resources": resources,
                  "renderer": M.renderer_metrics(header(), observations(), "off")}
        M.finalize_report(report)
        self.assertTrue(report["passed"])
        self.assertTrue(resources["rss_coverage_complete"])
        self.assertAlmostEqual(resources["mean_one_core_cpu_percent"], 6)
        self.assertEqual(resources["peak_summed_rss_bytes"], 300)

    def test_cpu_is_retained_when_renderer_or_rss_evidence_is_incomplete(self):
        rows = coalition_samples()
        rows[-1]["coalition"].update(tasks_started=3, tasks_exited=1)
        report = {"errors": ["renderer footer missing"], "ai_mode": "on", "platform": "darwin",
                  "resources": M.resource_metrics(rows)}
        M.finalize_report(report)
        self.assertFalse(report["passed"])
        self.assertTrue(report["incomplete"])
        self.assertAlmostEqual(report["resources"]["mean_one_core_cpu_percent"], 12)
        self.assertIsNone(report["gates"]["rss"])
        self.assertIn("exited", report["resource_scope"])

    def test_missing_or_regressed_coalition_counters_never_fall_back_to_pid_cpu(self):
        for mutate in [
            lambda rows: rows[-1].pop("coalition"),
            lambda rows: rows[-1]["coalition"].update(resource_id=4),
            lambda rows: rows[-1]["coalition"].update(cpu_ticks=0),
            lambda rows: rows[-1]["coalition"].update(tasks_started=1),
            lambda rows: rows[-1]["coalition"].update(tasks_exited=3),
            lambda rows: rows[-1]["coalition"].update(timebase_denom=0),
            lambda rows: rows[-1]["coalition"].update(timebase_numer=1),
            lambda rows: rows[-1]["coalition"].update(tasks_started=True),
        ]:
            rows = coalition_samples()
            mutate(rows)
            with self.assertRaises(M.Incomplete):
                M.resource_metrics(rows)

    def test_same_resource_with_different_jetsam_cannot_establish_isolation(self):
        root = M.Process(10, 1, 100, "/app", 0, 100, coalition=(3, 4))
        with self.assertRaises(M.Incomplete):
            M.isolated_mac_coalition(root, [root], (3, 9))
        helper = M.Process(11, 10, 101, "/helper", 0, 100, coalition=(3, 9))
        M.isolated_mac_coalition(root, [root, helper], (1, 2))

    def test_counter_live_count_mismatch_and_enumeration_churn_are_incomplete(self):
        rows = coalition_samples()
        rows[-1]["coalition"]["tasks_started"] = 3
        self.assertFalse(M.resource_metrics(rows)["rss_coverage_complete"])
        before = coalition_samples()[0]["coalition"]
        after = before | {"tasks_started": 3, "tasks_exited": 1}
        self.assertTrue(M.coalition_enumeration_limits(before, after, 2))
        self.assertEqual(M.coalition_enumeration_limits(before, before, 2), [])
        self.assertTrue(M.coalition_enumeration_limits(before, before, 1))

    def test_prefix_api_failure_or_partial_write_is_incomplete_without_native_load(self):
        collector = object.__new__(M.MacCollector)
        collector.coalition, collector.timebase = (3, 4), (125, 3)
        self.assertEqual(M.C.sizeof(M.MacCollector.CoalitionUsage), 32)
        self.assertEqual(M.MacCollector.CoalitionUsage.cpu_time.offset, 24)
        def read(cid, pointer, size):
            self.assertEqual((cid, size), (3, 32))
            value = M.C.cast(pointer, M.C.POINTER(M.MacCollector.CoalitionUsage)).contents
            value.tasks_started, value.tasks_exited = 2, 0
            value.time_nonempty, value.cpu_time = 10, 24_000_000
            return 0
        collector.read_coalition_usage = read
        self.assertEqual(collector.coalition_usage()["cpu_ticks"], 24_000_000)
        for failure in [lambda *_: -1, lambda *_: 0]:
            collector.read_coalition_usage = failure
            with self.assertRaises(M.Incomplete):
                collector.coalition_usage()
        with self.assertRaises(M.Incomplete):
            M.MacCollector.bind_coalition_usage(SimpleNamespace())
        class ShortPrefix(M.C.Structure):
            _fields_ = [("cpu_time", M.C.c_uint64)]
        with patch.object(M.MacCollector, "CoalitionUsage", ShortPrefix), self.assertRaises(M.Incomplete):
            M.MacCollector.bind_coalition_usage(SimpleNamespace())

    def test_resource_coalition_members_are_included_across_jetsam_ids(self):
        collector = object.__new__(M.MacCollector)
        collector.pid, collector.expected, collector.generation, collector.coalition, collector.uid = 10, "/app", 100, (3, 4), 501
        root = M.Process(10, 1, 100, "/app", 0, 100, coalition=(3, 4))
        helper = M.Process(11, 10, 101, "/helper", 0, 100, coalition=(3, 9))
        records = {row.pid: row for row in [root, helper]}
        collector.read = records.__getitem__
        collector.coalition_for = lambda pid: records[pid].coalition
        collector.bsd = lambda _: SimpleNamespace(credentials=[501])
        collector.parent_coalition = lambda: (1, 2)
        collector.coalition_usage = lambda: coalition_samples()[0]["coalition"]
        def pids(_kind, _uid, buffer, _size):
            if buffer is not None:
                buffer[0], buffer[1] = 10, 11
            return 8
        collector.lib = SimpleNamespace(proc_listpids=pids)
        with patch.object(M.C, "CDLL", side_effect=AssertionError("no native loading")):
            owned, limits = collector.snapshot()
        self.assertEqual([row.pid for row in owned], [10, 11])
        self.assertEqual(limits, [])
        calls = 0
        def reused_at_final_counter_fence(pid):
            nonlocal calls
            if pid == 10:
                calls += 1
                if calls == 4:
                    return M.replace(root, generation=999)
            return records[pid]
        collector.read = reused_at_final_counter_fence
        with self.assertRaises(M.Incomplete):
            collector.snapshot()

    def test_ai_on_without_lifetime_coverage_cannot_pass(self):
        for platform in ["darwin", "linux", "win32"]:
            report = {"errors": [], "ai_mode": "on", "platform": platform,
                      "resources": M.resource_metrics(native_samples()),
                      "renderer": {"age_p95_ms": 20, "interaction_p95_ms": 20}}
            M.finalize_report(report)
            self.assertFalse(report["passed"])
            self.assertTrue(report["incomplete"])
            self.assertIsNone(report["gates"]["cpu"])
            self.assertIsNone(report["gates"]["rss"])

    def test_zero_delta_cpu_is_valid_but_zero_or_nonmonotonic_lifetime_counters_are_not(self):
        rows = coalition_samples()
        for row in rows:
            row["coalition"]["cpu_ticks"] = 24_000_000
        self.assertEqual(M.resource_metrics(rows)["mean_one_core_cpu_percent"], 0)
        rows[0]["coalition"]["cpu_ticks"] = 0
        with self.assertRaises(M.Incomplete):
            M.resource_metrics(rows)
        rows = coalition_samples()
        rows[60]["coalition"].update(tasks_started=4, tasks_exited=2)
        with self.assertRaises(M.Incomplete):
            M.resource_metrics(rows)

    def test_actual_measure_path_retains_cpu_when_final_renderer_trace_is_missing(self):
        samples = coalition_samples()
        class FakeMacCollector:
            generation, started_at_unix_ms = 100, 500
            def __init__(self, *_):
                self.index = 0
            def snapshot(self):
                sample = samples[self.index]
                self.index += 1
                self.coalition_sample = sample["coalition"]
                return [M.Process(row["pid"], 0, row["generation"], "/app/helper", row["cpu_seconds"], row["rss_bytes"], row["footprint_bytes"]) for row in sample["processes"]], []
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            executable = root / "monitor"
            executable.write_bytes(b"fixture executable identity")
            trace = root / "probe.jsonl"
            trace.write_text(json.dumps(header() | {"narrative_preferences_path": str(root / "missing-preferences.json")}) + "\n")
            arguments = SimpleNamespace(pid=10, ai_mode="off", expected_executable=executable, probe_jsonl=trace)
            clock = [1.5]
            def sleep(seconds):
                clock[0] += seconds
            with patch.object(M, "MacCollector", FakeMacCollector), patch.object(M.sys, "platform", "darwin"), \
                    patch.object(M.time, "time", lambda: clock[0]), patch.object(M.time, "monotonic", lambda: clock[0]), \
                    patch.object(M.time, "sleep", sleep), patch.object(M.C, "CDLL", side_effect=AssertionError("no native loading")):
                report = M.measure(arguments)
        self.assertEqual(len(report["samples"]), 121)
        self.assertAlmostEqual(report["resources"]["mean_one_core_cpu_percent"], 12)
        self.assertTrue(any("footer missing" in error for error in report["errors"]))
        self.assertFalse(report["passed"])
        self.assertIsNone(report["gates"]["age"])


class OwnershipTests(unittest.TestCase):
    def test_shared_mac_parent_or_predating_coalition_member_is_rejected(self):
        root = M.Process(10, 1, 100, "/Applications/BatCave.app/Contents/MacOS/monitor", 0, 100, coalition=(3, 4))
        helper = M.Process(11, 1, 101, "/system/WebKit", 0, 100, coalition=(3, 4))
        M.isolated_mac_coalition(root, [root, helper], (1, 2))
        with self.assertRaises(M.Incomplete):
            M.isolated_mac_coalition(root, [root, helper], (3, 4))
        for generation in [99, 100]:
            anchor = M.Process(12, 1, generation, "/tools/shell", 0, 100, coalition=(3, 4))
            with self.assertRaises(M.Incomplete):
                M.isolated_mac_coalition(root, [root, helper, anchor], (1, 2))

    def test_same_name_or_executable_does_not_create_ownership(self):
        root = M.Process(1, 0, 10, "/app/monitor", 0, 100)
        child = M.Process(2, 1, 11, "/system/webkit", 0, 100)
        unrelated = M.Process(3, 0, 12, "/system/webkit", 0, 100)
        self.assertEqual([row.pid for row in M.verified_descendants([root, child, unrelated], 1)], [1, 2])

    def test_ambiguous_generation_or_missing_path_is_incomplete(self):
        root = M.Process(1, 0, 10, "/app/monitor", 0, 100)
        for generation, path in [(0, "/helper"), (9, "/helper"), (10, "/helper"), (11, ""), (11, "helper")]:
            with self.subTest(generation=generation, path=path), self.assertRaises(M.Incomplete):
                M.verified_descendants([root, M.Process(2, 1, generation, path, 0, 100)], 1)

    def test_root_reuse_and_wrong_executable_are_rejected(self):
        root = M.Process(1, 0, 10, "/app/monitor", 0, 100)
        with self.assertRaises(M.Incomplete):
            M.require_root(root, 1, "/app/monitor", 9)
        with self.assertRaises(M.Incomplete):
            M.require_root(root, 1, "/other/monitor", 10)

    def test_cyclic_ownership_is_not_admitted_from_numeric_parent_pids(self):
        root = M.Process(1, 2, 10, "/app/monitor", 0, 100)
        child = M.Process(2, 1, 11, "/helper", 0, 100)
        with self.assertRaises(M.Incomplete):
            M.verified_descendants([root, child], 1)

    def test_old_probe_cannot_bind_new_or_same_bucket_process_generation(self):
        for birth in [0, 1000, 1001]:
            with self.assertRaises(M.Incomplete):
                M.bind_probe_generation(birth, 1000)
        M.bind_probe_generation(999, 1000)

    def test_mach_absolute_ticks_are_converted_using_timebase(self):
        self.assertEqual(M.mach_seconds(24_000_000, 125, 3), 1)
        self.assertEqual(M.mach_seconds(1_000_000_000, 1, 1), 1)
        with self.assertRaises(M.Incomplete):
            M.mach_seconds(1, 1, 0)

    def test_linux_stat_parser_handles_parentheses_in_process_name(self):
        fields = ["S", "42"] + ["0"] * 9 + ["100", "50"] + ["0"] * 6 + ["900", "8192", "2"]
        self.assertEqual(M.LinuxCollector.parse_stat("51 (a ) process) " + " ".join(fields)), (51, 42, 900, 150, 2))


class GateTests(unittest.TestCase):
    def test_service_resources_are_separate_and_still_in_combined_budget(self):
        rows = native_samples()
        for row in rows:
            row["processes"][1]["role"] = "collector_service"
        metrics = M.resource_metrics(rows)
        self.assertAlmostEqual(metrics["process_sets"]["gui"]["mean_one_core_cpu_percent"], 4)
        self.assertAlmostEqual(metrics["process_sets"]["collector_service"]["mean_one_core_cpu_percent"], 4)
        self.assertAlmostEqual(metrics["mean_one_core_cpu_percent"], 8)
        self.assertEqual(metrics["peak_summed_rss_bytes"], 300)

    def test_cpu_is_one_core_and_rss_is_summed_across_owned_processes(self):
        metrics = M.resource_metrics(native_samples())
        self.assertAlmostEqual(metrics["mean_one_core_cpu_percent"], 8)
        self.assertEqual(metrics["peak_summed_rss_bytes"], 300)
        self.assertEqual(metrics["duration_seconds"], 120)

    def test_membership_churn_counter_regression_and_cadence_gap_are_incomplete(self):
        for mutate in [
            lambda rows: rows[-1]["processes"][0].update(generation=999),
            lambda rows: rows[-1]["processes"].pop(),
            lambda rows: rows[-1]["processes"][0].update(cpu_seconds=0),
            lambda rows: rows[-1]["processes"][0].update(cpu_seconds=float("nan")),
            lambda rows: rows[-1].update(elapsed_ms=151_000),
        ]:
            rows = native_samples()
            mutate(rows)
            with self.assertRaises(M.Incomplete):
                M.resource_metrics(rows)

    def test_full_renderer_window_meets_count_and_latency_contract(self):
        result = M.renderer_metrics(header(), observations(), "off")
        self.assertEqual(result["publication_samples"], 120)
        self.assertEqual(result["trusted_interactions"], 10)
        self.assertEqual(result["age_p95_ms"], 50)

    def test_renderer_hostile_matrix_rejects_missing_or_rebound_evidence(self):
        for mutate in [
            lambda rows: rows[0]["observation"].update(process_count=499),
            lambda rows: rows[0]["observation"].update(interval_ms=2000),
            lambda rows: rows[1]["observation"].update(publication_seq=1),
            lambda rows: rows[0].update(enhanced_narratives=True),
            lambda rows: rows[0]["observation"].update(age_ms=float("nan")),
            lambda rows: rows.pop(11),  # removes one of the ten interactions
        ]:
            rows = observations()
            mutate(rows)
            with self.assertRaises(M.Incomplete):
                M.renderer_metrics(header(), rows, "off")

    def test_latency_tail_is_not_hidden_by_average(self):
        rows = observations()
        publications = [row for row in rows if row["observation"]["kind"] == "publication"]
        for row in publications[-7:]:
            row["observation"]["age_ms"] = 3000
        self.assertEqual(M.renderer_metrics(header(), rows, "off")["age_p95_ms"], 3000)

    def test_probe_requires_complete_bound_footer(self):
        footer = {"kind": "footer", "elapsed_ms": 150000, "rejected_events": 0, "write_failed": False}
        for changes in [{}, {"elapsed_ms": 149999}, {"rejected_events": 1}, {"write_failed": True}]:
            value = footer | changes
            raw = b"\n".join(json.dumps(row).encode() for row in [header(), value]) + b"\n"
            if not changes:
                M.read_trace(io.BytesIO(raw), 10, True)
            else:
                with self.assertRaises(M.Incomplete):
                    M.read_trace(io.BytesIO(raw), 10, True)
        with self.assertRaises(M.Incomplete):
            M.read_trace(io.BytesIO(json.dumps(header()).encode() + b"\n"), 10, True)

    def test_ai_mode_is_read_from_preferences_and_missing_means_off_only(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "narrative-preferences.json"
            self.assertFalse(M.preferences(path, "off")["enabled"])
            with self.assertRaises(M.Incomplete):
                M.preferences(path, "on")
            path.write_text(json.dumps({"schema_version": 1, "enhanced_narratives": True}))
            self.assertTrue(M.preferences(path, "on")["enabled"])
            with self.assertRaises(M.Incomplete):
                M.preferences(path, "off")

    def test_failed_run_writes_incomplete_output_and_never_overwrites_it(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "report.json"
            arguments = ["measure-desktop.py", "--pid", "10", "--probe-jsonl", str(root / "probe.jsonl"),
                         "--output", str(output), "--expected-executable", str(root / "absent-monitor"), "--ai-mode", "off"]
            with patch.object(sys, "argv", arguments), redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
                self.assertEqual(M.main(), 1)
                original = output.read_bytes()
                self.assertEqual(M.main(), 2)
            result = json.loads(original)
            self.assertFalse(result["passed"])
            self.assertTrue(result["incomplete"])
            self.assertTrue(result["errors"])
            self.assertEqual(output.read_bytes(), original)


class WindowsServicePolicyTests(unittest.TestCase):
    expected = r"D:\Apps\BatCave Monitor\batcave-collector-service.exe"

    def config(self):
        return {"name": "BatCaveCollector", "image": f'"{self.expected}"', "account": "LocalSystem",
                "service_type": 16, "start_type": 2, "error_control": 1}

    def status(self):
        return {"service_type": 16, "state": 4, "pid": 73}

    def test_fixed_contract_matches_source_and_requires_exact_image_without_arguments(self):
        sources = Path(__file__).resolve().parents[1] / "src/BatCave.App/src-tauri/src/collector_service"
        self.assertIn(f'COLLECTOR_SERVICE_NAME: &str = "{M.SERVICE_NAME}"', (sources / "protocol.rs").read_text())
        self.assertIn(f'SERVICE_EXECUTABLE_NAME: &str = "{M.SERVICE_EXECUTABLE}"', (sources / "windows_provisioner.rs").read_text())
        self.assertEqual(M.service_process_id(self.config(), self.status(), self.expected), 73)
        for changes in [{"name": "Other"}, {"image": self.expected + " --other"},
                        {"image": f'"{self.expected}" --other'}, {"image": r"D:\Other\batcave-collector-service.exe"},
                        {"account": "user"}, {"service_type": 32}, {"start_type": 4}]:
            with self.assertRaises(M.Incomplete):
                M.service_process_id(self.config() | changes, self.status(), self.expected)

    def test_absence_is_distinct_from_access_denied_or_transitional_state(self):
        self.assertTrue(M.service_open_absent(1060))
        for error in [0, 5, 1072, 1722]:
            with self.assertRaises(M.Incomplete):
                M.service_open_absent(error)
        self.assertIsNone(M.service_process_id(self.config(), {"service_type": 16, "state": 1, "pid": 0}, self.expected))
        for state, pid in [(4, 0), (1, 73), (2, 73), (3, 73)]:
            with self.assertRaises(M.Incomplete):
                M.service_process_id(self.config(), {"service_type": 16, "state": state, "pid": pid}, self.expected)

    def collector(self, replies, record=None):
        collector = object.__new__(M.WindowsCollector)
        collector.service_expected, collector.service_identity = self.expected, None
        collector.query_service = lambda: next(replies)
        record = record or M.Process(73, 0, 100, self.expected, 1, 100)
        def read(pid, parent, validator):
            self.assertEqual(pid, record.pid)
            validator(record)
            return record
        collector.read = read
        return collector

    def test_service_binding_is_rechecked_while_process_handle_is_pinned(self):
        before = self.config(), self.status()
        collector = self.collector(iter([before, before]))
        record = collector.service_snapshot()
        self.assertEqual(record.role, "collector_service")
        self.assertEqual(collector.service_identity, (73, 100))
        changed = self.config(), self.status() | {"pid": 74}
        with self.assertRaises(M.Incomplete):
            self.collector(iter([before, changed])).service_snapshot()
        reused = self.collector(iter([before, before]))
        reused.service_identity = (73, 99)
        with self.assertRaises(M.Incomplete):
            reused.service_snapshot()
        wrong_image = M.Process(73, 0, 100, r"D:\Other\service.exe", 1, 100)
        with self.assertRaises(M.Incomplete):
            self.collector(iter([before, before]), wrong_image).service_snapshot()

    def test_service_absence_is_bracketed_and_never_inferred_from_enumeration(self):
        absent = self.collector(iter([None, None]))
        self.assertIsNone(absent.service_snapshot())
        self.assertEqual(absent.service_evidence["state"], "absent")
        with self.assertRaises(M.Incomplete):
            self.collector(iter([None, (self.config(), self.status())])).service_snapshot()


if __name__ == "__main__":
    unittest.main()
