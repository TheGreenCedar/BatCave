#!/usr/bin/env python3
"""Measure one already running native BatCave GUI. Never launch or terminate processes.

Start during the probe's 30-second warmup. Output is exclusive, including failed runs.
The 120-second gate uses one-core CPU and summed resident memory for the verified
owned process set. On macOS, cumulative resource-coalition CPU includes exited
helpers. Coalition lifetime churn makes sampled RSS coverage incomplete.
"""
from __future__ import annotations

import argparse
import ctypes as C
from dataclasses import asdict, dataclass, replace
import hashlib
import json
import math
import ntpath
import os
from pathlib import Path
import stat
import sys
import time

WARMUP_MS = 30_000
END_MS = 150_000
MAX_TRACE_BYTES = 4 * 1024 * 1024
# collector_service/protocol.rs and windows_provisioner.rs own these fixed values.
SERVICE_NAME = "BatCaveCollector"
SERVICE_EXECUTABLE = "batcave-collector-service.exe"


class Incomplete(RuntimeError):
    pass


@dataclass(frozen=True)
class Process:
    pid: int
    parent: int
    generation: int
    executable: str
    cpu_seconds: float
    rss_bytes: int
    footprint_bytes: int | None = None
    coalition: tuple[int, int] | None = None
    role: str = "gui"

    @property
    def identity(self):
        return self.pid, self.generation


def same_path(left, right):
    return os.path.normcase(os.path.realpath(left)) == os.path.normcase(os.path.realpath(right))


def mach_seconds(ticks, numerator, denominator):
    if numerator <= 0 or denominator <= 0 or ticks < 0:
        raise Incomplete("invalid Mach timebase or counter")
    return ticks * numerator / denominator / 1_000_000_000


def require_root(record, pid, expected, generation=None):
    if record is None or record.pid != pid or record.generation <= 0:
        raise Incomplete("root process generation unavailable")
    if generation is not None and record.generation != generation:
        raise Incomplete("root PID generation changed")
    if not record.executable or not same_path(record.executable, expected):
        raise Incomplete("root executable does not match --expected-executable")
    return record


def bind_probe_generation(start_upper_ms, probe_started_ms):
    if finite_number(start_upper_ms, "native birth bound", 1) >= probe_started_ms:
        raise Incomplete("native process generation does not precede the probe header")


def verified_descendants(records, root_pid):
    """Ancestry plus known executable/generation; ambiguous links are incomplete."""
    by_pid = {row.pid: row for row in records}
    if len(by_pid) != len(records):
        raise Incomplete("duplicate PID in process enumeration")
    owned, frontier = {root_pid}, [root_pid]
    while frontier:
        parent = by_pid[frontier.pop()]
        for child in records:
            if child.parent != parent.pid:
                continue
            if child.pid in owned:
                raise Incomplete("cycle in owned process ancestry")
            if child.generation <= parent.generation or not os.path.isabs(child.executable):
                raise Incomplete(f"unverified descendant generation or executable: {child.pid}")
            owned.add(child.pid)
            frontier.append(child.pid)
    return [by_pid[pid] for pid in sorted(owned)]


def isolated_mac_coalition(root, members, parent_coalition):
    if not root.coalition or not all(root.coalition) or not parent_coalition or not all(parent_coalition):
        raise Incomplete("root or parent coalition unavailable")
    if parent_coalition[0] == root.coalition[0]:
        raise Incomplete("app inherited its launching parent's coalition; launch through LaunchServices")
    for member in members:
        if not member.coalition or member.coalition[0] != root.coalition[0]:
            raise Incomplete("member left the app resource coalition")
        if member.pid != root.pid and member.generation <= root.generation:
            raise Incomplete("coalition contains a process predating the app; ownership is shared")


def validate_coalition_usage(value):
    if not isinstance(value, dict):
        raise Incomplete("resource coalition counters unavailable")
    for key in ("resource_id", "tasks_started", "tasks_exited", "cpu_ticks", "timebase_numer", "timebase_denom"):
        field = value.get(key)
        if type(field) is not int or not 0 <= field < (1 << 64) - 1:
            raise Incomplete(f"invalid resource coalition {key}")
    if (not value["resource_id"] or not value["tasks_started"] or not value["cpu_ticks"] or
            value["tasks_exited"] > value["tasks_started"] or
            not 0 < value["timebase_numer"] < 1 << 32 or not 0 < value["timebase_denom"] < 1 << 32):
        raise Incomplete("invalid resource coalition counts or timebase")


def validate_coalition_transition(before, after):
    for value in (before, after):
        validate_coalition_usage(value)
    for key in ("resource_id", "timebase_numer", "timebase_denom"):
        if before[key] != after[key]:
            raise Incomplete("resource coalition identity or timebase changed")
    for key in ("tasks_started", "tasks_exited", "cpu_ticks"):
        if after[key] < before[key]:
            raise Incomplete("resource coalition counter regressed")


def coalition_enumeration_limits(before, after, count):
    validate_coalition_transition(before, after)
    limits = []
    if any(before[key] != after[key] for key in ("tasks_started", "tasks_exited")):
        limits.append("resource coalition task lifetime changed during enumeration; RSS coverage incomplete")
    if after["tasks_started"] - after["tasks_exited"] != count:
        limits.append("resource coalition active count differs from sampled processes; RSS coverage incomplete")
    return limits


def service_open_absent(error):
    if error == 1060:  # ERROR_SERVICE_DOES_NOT_EXIST is the sole absence evidence.
        return True
    raise Incomplete(f"collector service status unknown: OpenService error {error}")


def service_process_id(config, status, expected_image):
    """Only exact SCM contract state can authorize service PID resource sampling."""
    image = config.get("image", "")
    if not isinstance(image, str):
        raise Incomplete("collector service image is not a string")
    if image.startswith('"') and image.endswith('"'):
        image = image[1:-1]
    if (not image or '"' in image or not ntpath.isabs(image) or
            any(part in (".", "..") for part in image.replace("/", "\\").split("\\")) or
            ntpath.normcase(image) != ntpath.normcase(expected_image) or
            config.get("name") != SERVICE_NAME or config.get("account") != "LocalSystem" or
            config.get("service_type") != 16 or config.get("start_type") != 2 or config.get("error_control") != 1 or
            status.get("service_type") != 16):
        raise Incomplete("collector service configuration does not match the installed contract")
    state, pid = status.get("state"), status.get("pid")
    if state == 1 and pid == 0:  # SERVICE_STOPPED, confirmed no service process.
        return None
    if state == 4 and type(pid) is int and pid > 0:  # SERVICE_RUNNING
        return pid
    raise Incomplete("collector service is transitional or has no authoritative process PID")


class MacCollector:
    # Apple XNU mach/coalition.h: stable prefix, excluding the evolving tail.
    # xnu-11215.81.4 and main: tasks_started, tasks_exited, time_nonempty, cpu_time.
    # https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/mach/coalition.h
    # The wrapper copies min(requested size, native struct size). cpu_time is Mach ticks.
    class CoalitionUsage(C.Structure):
        _fields_ = [(name, C.c_uint64) for name in ("tasks_started", "tasks_exited", "time_nonempty", "cpu_time")]

    @classmethod
    def bind_coalition_usage(cls, system):
        if (C.sizeof(cls.CoalitionUsage) != 32 or
                [getattr(cls.CoalitionUsage, name).offset for name in ("tasks_started", "tasks_exited", "time_nonempty", "cpu_time")] != [0, 8, 16, 24]):
            raise Incomplete("unexpected resource coalition prefix ABI")
        try:
            function = system.coalition_info_resource_usage
        except AttributeError as error:
            raise Incomplete("resource coalition cumulative CPU API unavailable") from error
        function.argtypes = [C.c_uint64, C.POINTER(cls.CoalitionUsage), C.c_size_t]
        function.restype = C.c_int
        return function

    # SDK sys/resource.h rusage_info_v2, sys/proc_info.h proc_bsdinfo.
    class Usage(C.Structure):
        _fields_ = [("uuid", C.c_ubyte * 16)] + [(name, C.c_uint64) for name in (
            "user", "system", "wakeups", "interrupts", "pageins", "wired", "rss",
            "footprint", "start", "exit", "child_user", "child_system", "child_wakeups",
            "child_interrupts", "child_pageins", "child_elapsed", "read", "write")]

    class Bsd(C.Structure):
        _fields_ = [("status", C.c_uint32 * 3), ("pid", C.c_uint32),
                    ("parent", C.c_uint32), ("credentials", C.c_uint32 * 7),
                    ("comm", C.c_char * 16), ("name", C.c_char * 32),
                    ("accounting", C.c_uint32 * 6), ("seconds", C.c_uint64),
                    ("microseconds", C.c_uint64)]

    def __init__(self, pid, expected):
        self.lib = C.CDLL("/usr/lib/libproc.dylib", use_errno=True)
        self.lib.proc_pidinfo.argtypes = [C.c_int, C.c_int, C.c_uint64, C.c_void_p, C.c_int]
        self.lib.proc_pid_rusage.argtypes = [C.c_int, C.c_int, C.c_void_p]
        self.lib.proc_pidpath.argtypes = [C.c_int, C.c_void_p, C.c_uint32]
        self.lib.proc_listpids.argtypes = [C.c_uint32, C.c_uint32, C.c_void_p, C.c_int]
        class Timebase(C.Structure):
            _fields_ = [("numer", C.c_uint32), ("denom", C.c_uint32)]
        timebase = Timebase()
        system = C.CDLL("/usr/lib/libSystem.B.dylib", use_errno=True)
        system.mach_timebase_info.argtypes = [C.POINTER(Timebase)]
        if system.mach_timebase_info(C.byref(timebase)) != 0 or not timebase.numer or not timebase.denom:
            raise Incomplete("Mach timebase unavailable")
        self.timebase = timebase.numer, timebase.denom
        if C.sizeof(self.Bsd) != 136 or C.sizeof(self.Usage) != 160:
            raise Incomplete("unexpected macOS process ABI")
        self.read_coalition_usage = self.bind_coalition_usage(system)
        self.pid, self.expected = pid, expected
        root = require_root(self.read(pid), pid, expected)
        self.generation, self.coalition = root.generation, root.coalition
        identity = self.bsd(pid)
        self.uid = identity.credentials[0]
        self.started_at_unix_ms = identity.seconds * 1000 + identity.microseconds / 1000
        if not self.coalition or not all(self.coalition):
            raise Incomplete("root coalition identity unavailable")
        isolated_mac_coalition(root, [root], self.parent_coalition())

    def coalition_usage(self):
        sentinel = (1 << 64) - 1
        value = self.CoalitionUsage(sentinel, sentinel, sentinel, sentinel)
        if self.read_coalition_usage(self.coalition[0], C.byref(value), C.sizeof(value)) != 0:
            raise Incomplete(f"resource coalition cumulative CPU unavailable: {C.get_errno()}")
        if value.time_nonempty == sentinel:
            raise Incomplete("resource coalition prefix was not fully returned")
        result = {"resource_id": self.coalition[0], "tasks_started": int(value.tasks_started),
                  "tasks_exited": int(value.tasks_exited), "cpu_ticks": int(value.cpu_time),
                  "timebase_numer": self.timebase[0], "timebase_denom": self.timebase[1]}
        validate_coalition_usage(result)
        return result

    def parent_coalition(self):
        before = self.bsd(self.pid)
        if before.parent <= 0:
            raise Incomplete("app launching parent is unavailable")
        coalition = self.coalition_for(before.parent)
        after = self.bsd(self.pid)
        if (before.parent, before.seconds, before.microseconds) != (after.parent, after.seconds, after.microseconds):
            raise Incomplete("app generation or parent changed during coalition check")
        return coalition

    def bsd(self, pid):
        value = self.Bsd()
        if self.lib.proc_pidinfo(pid, 3, 0, C.byref(value), C.sizeof(value)) != C.sizeof(value):
            raise OSError(C.get_errno(), f"process identity unavailable: {pid}")
        if value.pid != pid or not value.seconds or value.microseconds >= 1_000_000:
            raise Incomplete(f"invalid native birth: {pid}")
        return value

    def coalition_for(self, pid):
        # XNU proc_info_private.h: flavor 20, ids[2] followed by reserved[3].
        # https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/proc_info_private.h
        value = (C.c_uint64 * 5)()
        if self.lib.proc_pidinfo(pid, 20, 0, value, C.sizeof(value)) != 40:
            raise OSError(C.get_errno(), f"coalition unavailable: {pid}")
        return int(value[0]), int(value[1])

    def usage(self, pid):
        value = self.Usage()
        if self.lib.proc_pid_rusage(pid, 2, C.byref(value)) != 0 or not value.start:
            raise OSError(C.get_errno(), f"resource usage unavailable: {pid}")
        return value

    def read(self, pid):
        before, usage_before = self.bsd(pid), self.usage(pid)
        coalition = self.coalition_for(pid)
        path = C.create_string_buffer(4096)
        if self.lib.proc_pidpath(pid, path, len(path)) <= 0:
            raise OSError(C.get_errno(), f"executable unavailable: {pid}")
        usage, after = self.usage(pid), self.bsd(pid)
        if ((before.pid, before.seconds, before.microseconds) !=
                (after.pid, after.seconds, after.microseconds) or
                usage.start != usage_before.start or self.coalition_for(pid) != coalition):
            raise Incomplete(f"process changed during native probes: {pid}")
        return Process(pid, after.parent, usage.start, os.fsdecode(path.value),
                       mach_seconds(usage.user + usage.system, *self.timebase), usage.rss,
                       usage.footprint, coalition)

    def snapshot(self):
        initial = require_root(self.read(self.pid), self.pid, self.expected, self.generation)
        if initial.coalition != self.coalition:
            raise Incomplete("root coalition changed")
        before = self.coalition_usage()
        required = self.lib.proc_listpids(4, self.uid, None, 0)  # PROC_UID_ONLY
        if not 0 < required <= 4_000_000 or required % 4:
            raise Incomplete("native process enumeration failed")
        pids = (C.c_int * (required // 4 + 256))()
        copied = self.lib.proc_listpids(4, self.uid, pids, C.sizeof(pids))
        if copied <= 0 or copied >= C.sizeof(pids) or copied % 4:
            raise Incomplete("native process enumeration truncated")
        owned = []
        for pid in pids[:copied // 4]:
            if pid <= 0:
                continue
            try:
                if self.coalition_for(pid)[0] == self.coalition[0]:
                    if self.bsd(pid).credentials[0] != self.uid:
                        raise Incomplete(f"owned process UID changed: {pid}")
                    owned.append(self.read(pid))
            except OSError as error:
                # A process that exited before classification cannot supply a stable row.
                if error.errno in (3,):  # ESRCH
                    continue
                raise Incomplete(f"same-user process coverage unavailable: {pid}: {error}") from error
        root = next((row for row in owned if row.pid == self.pid), None)
        require_root(root, self.pid, self.expected, self.generation)
        isolated_mac_coalition(root, owned, self.parent_coalition())
        if require_root(self.read(self.pid), self.pid, self.expected, self.generation).coalition != self.coalition:
            raise Incomplete("root coalition changed during measurement")
        self.coalition_sample = self.coalition_usage()
        if require_root(self.read(self.pid), self.pid, self.expected, self.generation).coalition != self.coalition:
            raise Incomplete("root coalition changed during resource counter read")
        return owned, coalition_enumeration_limits(before, self.coalition_sample, len(owned))


class LinuxCollector:
    def __init__(self, pid, expected):
        self.pid, self.expected = pid, expected
        self.hz, self.page = os.sysconf("SC_CLK_TCK"), os.sysconf("SC_PAGE_SIZE")
        self.generation = require_root(self.read(pid), pid, expected).generation
        uptime_before = time.clock_gettime(time.CLOCK_BOOTTIME)
        # Upper edge of the collector's rounded start tick, using a bracketed boot epoch.
        self.started_at_unix_ms = (time.time() - uptime_before + (self.generation + 1) / self.hz) * 1000

    @staticmethod
    def parse_stat(text):
        pid, rest = text.split(" (", 1)
        fields = rest.rsplit(") ", 1)[1].split()
        return int(pid), int(fields[1]), int(fields[19]), int(fields[11]) + int(fields[12]), int(fields[21])

    def read(self, pid):
        base = Path("/proc") / str(pid)
        before = self.parse_stat((base / "stat").read_text())
        executable = os.readlink(base / "exe")
        after = self.parse_stat((base / "stat").read_text())
        if before[:3] != after[:3] or after[0] != pid or after[2] <= 0:
            raise Incomplete(f"process changed during /proc probes: {pid}")
        return Process(pid, after[1], after[2], executable, after[3] / self.hz, after[4] * self.page)

    def snapshot(self):
        require_root(self.read(self.pid), self.pid, self.expected, self.generation)
        records = []
        for path in Path("/proc").iterdir():
            if not path.name.isdigit():
                continue
            pid = int(path.name)
            try:
                records.append(self.read(pid))
            except FileNotFoundError:
                continue
            except PermissionError:
                # Keep ancestry facts; an unreadable descendant must fail ownership coverage.
                raw = self.parse_stat((path / "stat").read_text())
                records.append(Process(pid, raw[1], raw[2], "", 0, 0))
        owned = verified_descendants(records, self.pid)
        require_root(self.read(self.pid), self.pid, self.expected, self.generation)
        return owned, []


class WindowsCollector:
    def __init__(self, pid, expected):
        from ctypes import wintypes as W
        self.W = W
        self.kernel = C.WinDLL("kernel32", use_last_error=True)
        self.kernel.OpenProcess.restype = W.HANDLE
        self.kernel.CreateToolhelp32Snapshot.restype = W.HANDLE
        self.kernel.CloseHandle.argtypes = [W.HANDLE]
        self.kernel.QueryFullProcessImageNameW.argtypes = [W.HANDLE, W.DWORD, W.LPWSTR, C.POINTER(W.DWORD)]
        self.kernel.GetProcessTimes.argtypes = [W.HANDLE] + [C.POINTER(W.FILETIME)] * 4
        self.kernel.K32GetProcessMemoryInfo.argtypes = [W.HANDLE, C.c_void_p, W.DWORD]
        self.pid, self.expected = pid, expected
        self.generation = require_root(self.read(pid, 0), pid, expected).generation
        self.started_at_unix_ms = self.generation / 10_000 - 11644473600000
        self.service_expected = str(Path(expected).parent / SERVICE_EXECUTABLE)
        self.service_identity = None
        self.service_evidence = {"name": SERVICE_NAME, "state": "unknown"}
        self.manager = self.service = None
        self.scm = C.WinDLL("advapi32", use_last_error=True)
        self.scm.OpenSCManagerW.argtypes = [W.LPCWSTR, W.LPCWSTR, W.DWORD]
        self.scm.OpenSCManagerW.restype = W.HANDLE
        self.scm.OpenServiceW.argtypes = [W.HANDLE, W.LPCWSTR, W.DWORD]
        self.scm.OpenServiceW.restype = W.HANDLE
        self.scm.CloseServiceHandle.argtypes = [W.HANDLE]
        self.scm.QueryServiceConfigW.argtypes = [W.HANDLE, C.c_void_p, W.DWORD, C.POINTER(W.DWORD)]
        self.scm.QueryServiceStatusEx.argtypes = [W.HANDLE, C.c_int, C.c_void_p, W.DWORD, C.POINTER(W.DWORD)]
        self.manager = self.scm.OpenSCManagerW(None, None, 1)  # SC_MANAGER_CONNECT only.
        if not self.manager:
            raise Incomplete(f"collector SCM access unknown: {C.get_last_error()}")

    def close(self):
        for handle in (self.service, self.manager):
            if handle:
                self.scm.CloseServiceHandle(handle)
        self.service = self.manager = None

    def query_service(self):
        W = self.W
        if not self.service:
            self.service = self.scm.OpenServiceW(self.manager, SERVICE_NAME, 1 | 4)  # Query config/status.
            if not self.service:
                service_open_absent(C.get_last_error())
                return None
        class Config(C.Structure):
            _fields_ = [("type", W.DWORD), ("start", W.DWORD), ("error", W.DWORD),
                        ("image", C.c_void_p), ("group", C.c_void_p), ("tag", W.DWORD),
                        ("dependencies", C.c_void_p), ("account", C.c_void_p), ("display", C.c_void_p)]
        needed = W.DWORD()
        if self.scm.QueryServiceConfigW(self.service, None, 0, C.byref(needed)) or C.get_last_error() != 122 or not C.sizeof(Config) <= needed.value <= 65536:
            raise Incomplete("collector service configuration size unavailable")
        buffer = C.create_string_buffer(needed.value)
        if not self.scm.QueryServiceConfigW(self.service, buffer, len(buffer), C.byref(needed)):
            raise Incomplete(f"collector service configuration unavailable: {C.get_last_error()}")
        raw = C.cast(buffer, C.POINTER(Config)).contents
        def text(address):
            offset = (address or 0) - C.addressof(buffer)
            if offset < 0 or offset % C.sizeof(C.c_wchar) or offset >= len(buffer):
                raise Incomplete("collector service string pointer invalid")
            value = C.wstring_at(address, (len(buffer) - offset) // C.sizeof(C.c_wchar))
            if "\0" not in value:
                raise Incomplete("collector service string is unbounded")
            return value.split("\0", 1)[0]
        config = {"name": SERVICE_NAME, "image": text(raw.image), "account": text(raw.account),
                  "service_type": raw.type, "start_type": raw.start, "error_control": raw.error}
        status = (W.DWORD * 9)()
        if not self.scm.QueryServiceStatusEx(self.service, 0, status, C.sizeof(status), C.byref(needed)) or needed.value != C.sizeof(status):
            raise Incomplete(f"collector service status unavailable: {C.get_last_error()}")
        return config, {"service_type": status[0], "state": status[1], "pid": status[7]}

    def service_snapshot(self):
        self.service_evidence = {"name": SERVICE_NAME, "state": "unknown"}
        fence = int((time.time() + 11644473600) * 10_000_000)
        before = self.query_service()
        if before is None:
            if self.query_service() is not None:
                raise Incomplete("collector service appeared during absence check")
            self.service_evidence["state"] = "absent"
            return None
        config, status = before
        pid = service_process_id(config, status, self.service_expected)
        if pid is None:
            if self.query_service() != before:
                raise Incomplete("collector service changed during stopped-state check")
            self.service_evidence.update(state="stopped", image=config["image"])
            return None
        def revalidate(record):
            require_root(record, pid, self.service_expected)
            if record.generation >= fence or (self.service_identity is not None and record.identity != self.service_identity):
                raise Incomplete("collector service process generation changed")
            if self.query_service() != before:
                raise Incomplete("collector SCM binding changed while its process handle was pinned")
        record = self.read(pid, 0, revalidate)
        self.service_identity = record.identity
        self.service_evidence.update(state="running", image=record.executable, pid=pid, generation=record.generation)
        return replace(record, role="collector_service")

    def read(self, pid, parent, validator=None):
        W, kernel = self.W, self.kernel
        handle = kernel.OpenProcess(0x1000 | 0x10, False, pid)
        if not handle:
            raise OSError(C.get_last_error(), f"process open failed: {pid}")
        class Memory(C.Structure):
            _fields_ = [("cb", W.DWORD), ("faults", W.DWORD)] + [(name, C.c_size_t) for name in (
                "peak_rss", "rss", "peak_paged", "paged", "peak_nonpaged", "nonpaged", "pagefile", "peak_pagefile")]
        try:
            def times():
                values = [W.FILETIME() for _ in range(4)]
                if not kernel.GetProcessTimes(handle, *(C.byref(value) for value in values)):
                    raise OSError(C.get_last_error(), "GetProcessTimes failed")
                return [(value.dwHighDateTime << 32) | value.dwLowDateTime for value in values]
            before = times()
            path, length = C.create_unicode_buffer(32768), W.DWORD(32768)
            memory = Memory()
            memory.cb = C.sizeof(memory)
            if not kernel.QueryFullProcessImageNameW(handle, 0, path, C.byref(length)) or not kernel.K32GetProcessMemoryInfo(handle, C.byref(memory), memory.cb):
                raise OSError(C.get_last_error(), "native executable/memory unavailable")
            after = times()
            if before[0] != after[0] or not before[0] or after[1]:
                raise Incomplete(f"process generation changed or exited: {pid}")
            record = Process(pid, parent, after[0], path.value, (after[2] + after[3]) / 10_000_000, memory.rss)
            if validator:
                validator(record)  # SCM is rechecked while this exact process handle stays open.
                final = times()
                if final[0] != after[0] or final[1]:
                    raise Incomplete(f"service process exited during SCM revalidation: {pid}")
            return record
        finally:
            kernel.CloseHandle(handle)

    def snapshot(self):
        W, kernel = self.W, self.kernel
        class Entry(C.Structure):
            _fields_ = [("size", W.DWORD), ("usage", W.DWORD), ("pid", W.DWORD),
                        ("heap", C.c_size_t), ("module", W.DWORD), ("threads", W.DWORD),
                        ("parent", W.DWORD), ("priority", W.LONG), ("flags", W.DWORD), ("exe", W.WCHAR * 260)]
        kernel.Process32FirstW.argtypes = kernel.Process32NextW.argtypes = [W.HANDLE, C.POINTER(Entry)]
        require_root(self.read(self.pid, 0), self.pid, self.expected, self.generation)
        fence = int((time.time() + 11644473600) * 10_000_000)
        handle = kernel.CreateToolhelp32Snapshot(2, 0)
        if handle in (None, C.c_void_p(-1).value):
            raise Incomplete("Toolhelp snapshot failed")
        records = []
        try:
            entry = Entry()
            entry.size = C.sizeof(entry)
            more = kernel.Process32FirstW(handle, C.byref(entry))
            while more:
                try:
                    record = self.read(entry.pid, entry.parent)
                    if record.generation >= fence:
                        record = Process(entry.pid, entry.parent, 0, "", 0, 0)
                except OSError:
                    record = Process(entry.pid, entry.parent, 0, "", 0, 0)
                records.append(record)
                more = kernel.Process32NextW(handle, C.byref(entry))
            if C.get_last_error() != 18:  # ERROR_NO_MORE_FILES
                raise Incomplete("Toolhelp enumeration incomplete")
        finally:
            kernel.CloseHandle(handle)
        owned = verified_descendants(records, self.pid)
        service = self.service_snapshot()
        if service:
            if service.pid in {row.pid for row in owned}:
                raise Incomplete("collector service PID overlaps the GUI process tree")
            owned.append(service)
        require_root(self.read(self.pid, 0), self.pid, self.expected, self.generation)
        return owned, []


def finite_number(value, name, minimum=0):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value < minimum:
        raise Incomplete(f"invalid {name}")
    return value


def percentile95(values):
    if not values:
        raise Incomplete("no samples for percentile")
    return sorted(values)[math.ceil(len(values) * 0.95) - 1]


def read_trace(file, pid, complete):
    file.seek(0)
    raw = file.read(MAX_TRACE_BYTES + 1)
    if len(raw) > MAX_TRACE_BYTES or (complete and not raw.endswith(b"\n")):
        raise Incomplete("probe trace is oversized or truncated")
    lines = raw.splitlines()
    if not complete:
        lines = lines[:1]
    try:
        rows = [json.loads(line) for line in lines]
    except (ValueError, UnicodeDecodeError) as error:
        raise Incomplete("invalid probe JSONL") from error
    if any(not isinstance(row, dict) for row in rows):
        raise Incomplete("probe rows must be JSON objects")
    if not rows or rows[0].get("kind") != "header" or rows[0].get("schema_version") != 1 or rows[0].get("pid") != pid:
        raise Incomplete("probe header does not bind requested PID")
    header = rows[0]
    if header.get("warmup_seconds") != 30 or header.get("measurement_seconds") != 120:
        raise Incomplete("probe duration contract mismatch")
    if header.get("interaction_boundary") != "trusted_dom_event_to_second_animation_frame" or header.get("publication_boundary") != "runtime_publication_to_renderer_second_animation_frame":
        raise Incomplete("probe renderer boundary unverified")
    if not isinstance(header.get("release_identity"), dict) or not header["release_identity"].get("app_version"):
        raise Incomplete("probe release identity missing")
    finite_number(header.get("started_at_unix_ms"), "probe start", 1)
    if complete:
        if len(rows) < 2 or rows[-1].get("kind") != "footer":
            raise Incomplete("probe footer missing")
        footer = rows[-1]
        if finite_number(footer.get("elapsed_ms"), "footer elapsed") < END_MS or type(footer.get("rejected_events")) is not int or footer["rejected_events"] != 0 or footer.get("write_failed") is not False:
            raise Incomplete("probe ended early or lost events")
        if any(row.get("kind") != "observation" for row in rows[1:-1]):
            raise Incomplete("unexpected probe row")
    return header, rows[1:-1] if complete else []


def preferences(path, mode):
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        if mode == "off":
            return {"enabled": False, "source": "missing_file_default"}
        raise Incomplete("AI-on requested but preferences file is missing")
    if not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1 or metadata.st_size > 4096:
        raise Incomplete("preferences are not a bounded regular file")
    value = json.loads(path.read_bytes())
    after = path.lstat()
    if (metadata.st_dev, metadata.st_ino, metadata.st_size, metadata.st_mtime_ns) != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns):
        raise Incomplete("preferences changed while reading")
    if value.get("schema_version") != 1 or type(value.get("enhanced_narratives")) is not bool:
        raise Incomplete("preferences schema invalid")
    enabled = value["enhanced_narratives"]
    if enabled != (mode == "on"):
        raise Incomplete("AI preference does not match --ai-mode")
    return {"enabled": enabled, "source": "preferences_file"}


def renderer_metrics(header, observations, mode):
    wanted = mode == "on"
    if header.get("enhanced_narratives") is not wanted:
        raise Incomplete("probe initial AI mode mismatch")
    ages, interactions, times, counts, sequences = [], [], [], [], set()
    previous_seq = 0
    for row in observations:
        elapsed = finite_number(row.get("elapsed_ms"), "observation elapsed")
        if not WARMUP_MS <= elapsed <= END_MS:
            continue
        if row.get("enhanced_narratives") is not wanted:
            raise Incomplete("AI mode changed during measurement")
        observation = row.get("observation", {})
        if observation.get("kind") == "publication":
            sequence = observation.get("publication_seq")
            if type(sequence) is not int or sequence <= previous_seq or sequence in sequences:
                raise Incomplete("duplicate or unordered renderer publication")
            if observation.get("interval_ms") != 1000:
                raise Incomplete("renderer did not use one-second sampling")
            previous_seq = sequence
            sequences.add(sequence)
            ages.append(finite_number(observation.get("age_ms"), "publication age"))
            counts.append(finite_number(observation.get("process_count"), "process count"))
            times.append(elapsed)
        elif observation.get("kind") == "interaction":
            interactions.append(finite_number(observation.get("duration_ms"), "interaction duration"))
        else:
            raise Incomplete("unknown renderer observation")
    if len(ages) < 100 or len(interactions) < 10 or min(counts) < 500:
        raise Incomplete("requires >=100 renderer samples, >=10 trusted interactions, and >=500 processes throughout")
    if times[0] > WARMUP_MS + 2000 or times[-1] < END_MS - 2000 or max(b - a for a, b in zip(times, times[1:])) > 2500:
        raise Incomplete("renderer publication coverage has gaps")
    return {"publication_samples": len(ages), "trusted_interactions": len(interactions),
            "minimum_process_count": min(counts), "age_p95_ms": percentile95(ages),
            "interaction_p95_ms": percentile95(interactions)}


def resource_metrics(samples):
    if len(samples) < 120 or samples[0]["elapsed_ms"] > WARMUP_MS + 500 or samples[-1]["elapsed_ms"] < END_MS:
        raise Incomplete("native measurement window incomplete")
    if any("coalition" in sample for sample in samples):
        return coalition_resource_metrics(samples)
    identities = {row["pid"]: row["generation"] for row in samples[0]["processes"]}
    if not identities:
        raise Incomplete("owned native process set is empty")
    for sample in samples:
        if len({row["pid"] for row in sample["processes"]}) != len(sample["processes"]):
            raise Incomplete("duplicate owned native PID")
        for row in sample["processes"]:
            finite_number(row["generation"], "native generation", 1)
            finite_number(row["cpu_seconds"], "native CPU counter")
            finite_number(row["rss_bytes"], "native resident memory")
    cpu_seconds, duration, role_cpu = 0.0, 0.0, {}
    for previous, current in zip(samples, samples[1:]):
        if {row["pid"]: row["generation"] for row in current["processes"]} != identities:
            raise Incomplete("owned process membership changed; exit CPU cannot be fully accounted")
        dt = (current["elapsed_ms"] - previous["elapsed_ms"]) / 1000
        if not 0 < dt <= 1.5:
            raise Incomplete("native sample cadence has gaps")
        before = {row["pid"]: row for row in previous["processes"]}
        for row in current["processes"]:
            role = row.get("role", "gui")
            if role not in ("gui", "collector_service") or role != before[row["pid"]].get("role", "gui"):
                raise Incomplete("native process ownership role changed")
            delta = row["cpu_seconds"] - before[row["pid"]]["cpu_seconds"]
            if delta < 0:
                raise Incomplete("native CPU counter regressed")
            cpu_seconds += delta
            role_cpu[role] = role_cpu.get(role, 0.0) + delta
        duration += dt
    if duration < 119.5:
        raise Incomplete("native resource window is too short")
    rss = [sum(row["rss_bytes"] for row in sample["processes"]) for sample in samples]
    footprints = [sum(row["footprint_bytes"] for row in sample["processes"]) for sample in samples
                  if all(row["footprint_bytes"] is not None for row in sample["processes"])]
    return {"native_samples": len(samples), "duration_seconds": duration,
            "owned_process_count": len(identities), "mean_one_core_cpu_percent": cpu_seconds / duration * 100,
            "peak_summed_rss_bytes": max(rss), "peak_summed_footprint_bytes": max(footprints) if footprints else None,
            "process_sets": {role: {"mean_one_core_cpu_percent": cpu / duration * 100,
                "peak_summed_rss_bytes": max(sum(row["rss_bytes"] for row in sample["processes"] if row.get("role", "gui") == role) for sample in samples)}
                for role, cpu in role_cpu.items()}}


def coalition_resource_metrics(samples):
    """Cumulative coalition CPU is independent of sampled process memory coverage."""
    first = samples[0].get("coalition")
    validate_coalition_usage(first)
    duration, rss, footprints, seen, errors = 0.0, [], [], set(), []
    first_ids = {(row["pid"], row["generation"]) for row in samples[0]["processes"]}
    previous = None
    for sample in samples:
        current = sample.get("coalition")
        validate_coalition_usage(current)
        finite_number(sample["elapsed_ms"], "native sample time")
        errors.extend(sample.get("coverage_errors", []))
        validate_coalition_transition(first, current)
        errors.extend(coalition_enumeration_limits(current, current, len(sample["processes"])))
        if any(current[key] != first[key] for key in ("tasks_started", "tasks_exited")):
            errors.append("resource coalition task lifetime changed during measurement; RSS coverage incomplete")
        if previous is not None:
            dt = (sample["elapsed_ms"] - previous["elapsed_ms"]) / 1000
            if not 0 < dt <= 1.5:
                raise Incomplete("native sample cadence has gaps")
            validate_coalition_transition(previous["coalition"], current)
            duration += dt
        previous = sample
        # Missing RSS/generation evidence must not erase independently valid cumulative CPU.
        try:
            records = sample["processes"]
            ids = {(row["pid"], row["generation"]) for row in records}
            if not ids or len({row["pid"] for row in records}) != len(records):
                raise Incomplete("owned native process set is empty or duplicated")
            if ids != first_ids:
                errors.append("sampled process membership changed; RSS lifetime coverage incomplete")
            seen.update(ids)
            for row in records:
                finite_number(row["generation"], "native generation", 1)
                finite_number(row["rss_bytes"], "native resident memory")
                if row.get("role", "gui") != "gui":
                    raise Incomplete("non-GUI process role in app resource coalition")
            rss.append(sum(row["rss_bytes"] for row in records))
            if all(row.get("footprint_bytes") is not None for row in records):
                footprints.append(sum(finite_number(row["footprint_bytes"], "native footprint") for row in records))
        except (KeyError, TypeError, Incomplete) as error:
            errors.append(f"sampled process memory unavailable: {error}")
    if duration < 119.5:
        raise Incomplete("native resource window is too short")
    last = samples[-1]["coalition"]
    cpu_seconds = mach_seconds(last["cpu_ticks"] - first["cpu_ticks"], first["timebase_numer"], first["timebase_denom"])
    errors = list(dict.fromkeys(errors))
    return {"native_samples": len(samples), "duration_seconds": duration,
            "owned_process_count": len(seen), "mean_one_core_cpu_percent": cpu_seconds / duration * 100,
            "peak_summed_rss_bytes": max(rss) if rss else None,
            "peak_summed_footprint_bytes": max(footprints) if footprints else None,
            "cpu_coverage_complete": True, "rss_coverage_complete": not errors,
            "cpu_scope": "isolated resource coalition cumulative CPU, including exited members; excludes services outside that coalition",
            "rss_scope": "sampled resident memory of live resource-coalition processes; departed helper peaks are not recoverable",
            "resource_id": first["resource_id"],
            "tasks_started_during_measurement": last["tasks_started"] - first["tasks_started"],
            "tasks_exited_during_measurement": last["tasks_exited"] - first["tasks_exited"],
            "coverage_errors": errors}


def finalize_report(report):
    """Retain independent metrics when another boundary fails; incomplete gates are null."""
    resources, renderer = report.get("resources"), report.get("renderer")
    errors = report["errors"]
    cpu_covered = resources is not None
    if resources:
        errors.extend(item for item in resources.get("coverage_errors", []) if item not in errors)
        lifetime_cpu = resources.get("cpu_coverage_complete") is True
        if (report["platform"] == "darwin" or report["ai_mode"] == "on") and not lifetime_cpu:
            errors.append("resource lifetime coverage unavailable; a whole-app resource pass is not established")
            cpu_covered = False
        report["resource_scope"] = (resources["cpu_scope"] + "; " + resources["rss_scope"] if lifetime_cpu else
                                    "sampled native app-owned processes; processes entirely between polls are not observable")
    report["gates"] = {
        "age": renderer["age_p95_ms"] <= 2000 if renderer else None,
        "interaction": renderer["interaction_p95_ms"] <= 100 if renderer else None,
        "cpu": resources["mean_one_core_cpu_percent"] <= 10 if cpu_covered else None,
        "rss": resources["peak_summed_rss_bytes"] <= 512 * 1024 * 1024
            if cpu_covered and resources.get("rss_coverage_complete", True) else None,
    }
    report["incomplete"] = bool(errors) or not resources or not renderer
    report["passed"] = not report["incomplete"] and all(value is True for value in report["gates"].values())


def file_identity(file):
    info = os.fstat(file.fileno())
    if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
        raise Incomplete("probe must be a single-link regular file")
    return info.st_dev, info.st_ino


def measure(args):
    report = {"schema_version": 1, "passed": False, "incomplete": True, "errors": [],
              "platform": sys.platform, "pid": args.pid, "ai_mode": args.ai_mode, "samples": [],
              "interaction_scope": "trusted DOM event to second animation frame; does not prove asynchronous inspector completion",
              "resource_scope": "unavailable; no complete native resource measurement"}
    collector = None
    if sys.platform == "win32":
        report["collector_service"] = {"name": SERVICE_NAME, "state": "unknown"}
    try:
        expected = args.expected_executable.resolve(strict=True)
        if not expected.is_file():
            raise Incomplete("expected executable is not a file")
        report["expected_executable"] = str(expected)
        digest = hashlib.sha256(expected.read_bytes()).hexdigest()
        report["executable_sha256"] = digest
        fd = os.open(args.probe_jsonl, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
        with os.fdopen(fd, "rb") as trace:
            identity = file_identity(trace)
            header, _ = read_trace(trace, args.pid, False)
            report["probe_header"] = header
            preference_path = header.get("narrative_preferences_path")
            if not isinstance(preference_path, str) or not Path(preference_path).is_absolute():
                raise Incomplete("probe did not identify actual narrative preferences path")
            preference_path = Path(preference_path)
            report["ai_verification"] = preferences(preference_path, args.ai_mode)
            if header.get("enhanced_narratives") is not (args.ai_mode == "on"):
                raise Incomplete("host AI mode differs from requested mode")
            wall_anchor, mono_anchor = time.time(), time.monotonic()
            elapsed_start = wall_anchor * 1000 - header["started_at_unix_ms"]
            if not 0 <= elapsed_start < WARMUP_MS - 1000:
                raise Incomplete("attach during warmup, at least one second before measurement")
            collector_type = {"darwin": MacCollector, "linux": LinuxCollector, "win32": WindowsCollector}.get(sys.platform)
            if collector_type is None:
                raise Incomplete("unsupported native platform")
            collector = collector_type(args.pid, str(expected))
            bind_probe_generation(collector.started_at_unix_ms, header["started_at_unix_ms"])
            report["root_generation"] = collector.generation
            report["root_started_at_unix_ms_upper_bound"] = collector.started_at_unix_ms
            report["ownership_method"] = "native_resource_coalition_generation_and_lifetime_counters" if sys.platform == "darwin" else "verified_ancestry_executable_and_generation"
            target_ms = WARMUP_MS
            while target_ms <= END_MS:
                deadline = mono_anchor + (target_ms - elapsed_start) / 1000
                time.sleep(max(0, deadline - time.monotonic()))
                owned, limitations = collector.snapshot()
                elapsed = elapsed_start + (time.monotonic() - mono_anchor) * 1000
                if abs((time.time() - wall_anchor) - (time.monotonic() - mono_anchor)) > 0.25:
                    raise Incomplete("wall clock changed; probe window alignment lost")
                if file_identity(trace) != identity or os.stat(args.probe_jsonl, follow_symlinks=False).st_ino != identity[1]:
                    raise Incomplete("probe file was replaced")
                preferences(preference_path, args.ai_mode)
                sample = {"elapsed_ms": elapsed, "processes": [asdict(row) for row in owned]}
                if isinstance(collector, MacCollector):
                    sample["coalition"] = collector.coalition_sample
                    sample["coverage_errors"] = limitations
                report["samples"].append(sample)
                if limitations:
                    report["errors"].extend(item for item in limitations if item not in report["errors"])
                target_ms += 1000
            # Compute resources before renderer/footer validation so valid CPU evidence survives
            # an independent UI coverage failure. Coverage errors still prevent an overall pass.
            report["resources"] = resource_metrics(report["samples"])
            time.sleep(0.25)  # Let the app's 150-second footer flush.
            final_header, observations = read_trace(trace, args.pid, True)
            if final_header != header:
                raise Incomplete("probe header changed")
            report["renderer"] = renderer_metrics(header, observations, args.ai_mode)
            if hashlib.sha256(expected.read_bytes()).hexdigest() != digest:
                raise Incomplete("expected executable changed during measurement")
    except (Exception, KeyboardInterrupt) as error:
        # Every failed boundary produces an incomplete artifact, including platform API errors.
        report["errors"].append(f"{type(error).__name__}: {error}")
    finally:
        if collector is not None and hasattr(collector, "service_evidence"):
            report["collector_service"] = collector.service_evidence
            collector.close()
    finalize_report(report)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--probe-jsonl", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--expected-executable", type=Path, required=True)
    parser.add_argument("--ai-mode", choices=("off", "on"), required=True)
    args = parser.parse_args()
    if args.pid <= 0 or not all(path.is_absolute() for path in (args.probe_jsonl, args.output, args.expected_executable)):
        parser.error("PID must be positive and all paths must be absolute")
    try:
        with args.output.open("x", encoding="utf-8") as output:
            report = measure(args)
            json.dump(report, output, indent=2, allow_nan=False)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
    except OSError as error:
        print(f"Cannot create exclusive measurement output: {error}", file=sys.stderr)
        return 2
    print(json.dumps({"passed": report["passed"], "incomplete": report["incomplete"], "errors": report["errors"], "output": str(args.output)}))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
