"""Fixed deb GUI observation inside the Node-owned standard-user transient unit.

No browser fixture, proof CLI, root GUI, external display, or sandbox override.
The enclosing cgroup supervisor is authoritative for descendant settlement.
"""

import ctypes as C
import hashlib
import json
import os
from pathlib import Path
import re
import select
import stat
import struct
import subprocess
import sys
import tarfile
import time


GUI = Path("/usr/bin/batcave-monitor")
PYTHON = "/usr/bin/python3.10"
IMPORT = "/usr/bin/import-im6.q16"
IDENTIFY = "/usr/bin/identify-im6.q16"
TITLE = "BatCave Monitor"
MAX_BYTES = 512 * 1024 * 1024
BASE_ENV = {"PATH": "/usr/bin:/bin", "LANG": "C", "LC_ALL": "C"}


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def digest_file(path):
    info = path.stat()
    require(stat.S_ISREG(info.st_mode) and 0 < info.st_size <= MAX_BYTES, "invalid GUI bytes")
    with path.open("rb") as stream:
        digest = "sha256:" + hash_stream(stream)
    after = path.stat()
    require((info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns) == (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns), "GUI file changed")
    return digest


def hash_stream(stream):
    digest = hashlib.sha256()
    for chunk in iter(lambda: stream.read(1024 * 1024), b""):
        digest.update(chunk)
    return digest.hexdigest()


def deb_gui_digest(artifact):
    # Stream one fixed regular member. Never extract arbitrary archive paths.
    child = subprocess.Popen(["/usr/bin/dpkg-deb", "--fsys-tarfile", str(artifact)], stdout=subprocess.PIPE)
    found = None
    try:
        with tarfile.open(fileobj=child.stdout, mode="r|") as archive:
            for member in archive:
                if member.name in ("./usr/bin/batcave-monitor", "usr/bin/batcave-monitor"):
                    require(found is None and member.isreg() and 0 < member.size <= MAX_BYTES, "invalid deb GUI member")
                    found = "sha256:" + hash_stream(archive.extractfile(member))
        child.stdout.close()
        require(child.wait(timeout=10) == 0 and found is not None, "deb GUI member unavailable")
        return found
    finally:
        if child.poll() is None:
            child.kill()
            child.wait(timeout=5)


def process_generation(text):
    require(")" in text, "invalid process stat")
    fields = text.rsplit(")", 1)[1].split()
    require(len(fields) >= 20 and fields[19].isdecimal() and int(fields[19]) > 0, "invalid process generation")
    return fields[19]


def process_identity(pid, unit, expected_digest):
    proc = Path("/proc") / str(pid)
    # comm may contain spaces or parentheses; starttime is field 22 after its final ')'.
    before = process_generation((proc / "stat").read_text())
    require(proc.stat().st_uid == os.getuid(), "GUI PID is not current user")
    cgroups = (proc / "cgroup").read_text().splitlines()
    require(any(line.split(":", 2)[-1].split("/")[-1] == unit for line in cgroups), "GUI PID escaped owned unit")
    # This is the one authorized procfs executable link, not an arbitrary filesystem link.
    executable = proc / "exe"
    require(os.readlink(executable) == str(GUI), "window PID is not installed GUI")
    require((executable.stat().st_dev, executable.stat().st_ino) == (GUI.stat().st_dev, GUI.stat().st_ino), "GUI executable inode changed")
    require(digest_file(executable) == expected_digest, "GUI process bytes differ from public deb")
    after = process_generation((proc / "stat").read_text())
    require(before == after and int(before) > 0, "GUI process generation changed")
    return before


class WindowAttributes(C.Structure):
    _fields_ = [("x", C.c_int), ("y", C.c_int), ("width", C.c_int), ("height", C.c_int),
                ("border_width", C.c_int), ("depth", C.c_int), ("visual", C.c_void_p),
                ("root", C.c_ulong), ("class_", C.c_int), ("bit_gravity", C.c_int),
                ("win_gravity", C.c_int), ("backing_store", C.c_int),
                ("backing_planes", C.c_ulong), ("backing_pixel", C.c_ulong),
                ("save_under", C.c_int), ("colormap", C.c_ulong), ("map_installed", C.c_int),
                ("map_state", C.c_int), ("all_event_masks", C.c_long),
                ("your_event_mask", C.c_long), ("do_not_propagate_mask", C.c_long),
                ("override_redirect", C.c_int), ("screen", C.c_void_p)]


class X11:
    def __init__(self):
        self.lib = C.CDLL("libX11.so.6")
        signatures = {
            "XOpenDisplay": ([C.c_char_p], C.c_void_p),
            "XDefaultRootWindow": ([C.c_void_p], C.c_ulong),
            "XQueryTree": ([C.c_void_p, C.c_ulong, C.POINTER(C.c_ulong), C.POINTER(C.c_ulong), C.POINTER(C.POINTER(C.c_ulong)), C.POINTER(C.c_uint)], C.c_int),
            "XFetchName": ([C.c_void_p, C.c_ulong, C.POINTER(C.c_void_p)], C.c_int),
            "XGetWindowAttributes": ([C.c_void_p, C.c_ulong, C.POINTER(WindowAttributes)], C.c_int),
            "XInternAtom": ([C.c_void_p, C.c_char_p, C.c_int], C.c_ulong),
            "XGetWindowProperty": ([C.c_void_p, C.c_ulong, C.c_ulong, C.c_long, C.c_long, C.c_int, C.c_ulong, C.POINTER(C.c_ulong), C.POINTER(C.c_int), C.POINTER(C.c_ulong), C.POINTER(C.c_ulong), C.POINTER(C.c_void_p)], C.c_int),
            "XFree": ([C.c_void_p], C.c_int),
            "XCloseDisplay": ([C.c_void_p], C.c_int),
        }
        for name, (args, result) in signatures.items():
            getattr(self.lib, name).argtypes = args
            getattr(self.lib, name).restype = result
        self.display = self.lib.XOpenDisplay(None)
        require(self.display, "owned X display unavailable")

    def windows(self):
        root = self.lib.XDefaultRootWindow(self.display)
        parent, returned_root = C.c_ulong(), C.c_ulong()
        children, count = C.POINTER(C.c_ulong)(), C.c_uint()
        require(self.lib.XQueryTree(self.display, root, C.byref(returned_root), C.byref(parent), C.byref(children), C.byref(count)), "owned X tree unavailable")
        try:
            require(count.value <= 4096, "owned X tree exceeded boundary")
            return [children[index] for index in range(count.value)]
        finally:
            if children:
                self.lib.XFree(children)

    def window(self, window):
        attributes, name = WindowAttributes(), C.c_void_p()
        require(self.lib.XGetWindowAttributes(self.display, window, C.byref(attributes)), "window vanished")
        self.lib.XFetchName(self.display, window, C.byref(name))
        try:
            title = C.string_at(name).decode("utf-8", "strict") if name.value else ""
        finally:
            if name.value:
                self.lib.XFree(name)
        atom = self.lib.XInternAtom(self.display, b"_NET_WM_PID", 1)
        actual_type, format_, count, remaining, data = C.c_ulong(), C.c_int(), C.c_ulong(), C.c_ulong(), C.c_void_p()
        pid = None
        if atom and self.lib.XGetWindowProperty(self.display, window, atom, 0, 1, 0, 6, C.byref(actual_type), C.byref(format_), C.byref(count), C.byref(remaining), C.byref(data)) == 0:
            try:
                if actual_type.value == 6 and format_.value == 32 and count.value == 1 and remaining.value == 0 and data.value:
                    pid = C.cast(data, C.POINTER(C.c_ulong))[0]
            finally:
                if data.value:
                    self.lib.XFree(data)
        return title, pid, attributes

    def close(self):
        self.lib.XCloseDisplay(self.display)


def observe(workspace, artifact, unit):
    public_digest = digest_file(artifact)
    expected = deb_gui_digest(artifact)
    require(not GUI.is_symlink() and digest_file(GUI) == expected, "installed GUI differs from exact deb member")
    application = subprocess.Popen([str(GUI)], cwd=workspace, env=dict(os.environ))
    x = X11()
    try:
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline:
            require(application.poll() is None, "packaged GUI exited before rendered observation")
            for window in x.windows():
                title, pid, attributes = x.window(window)
                if title != TITLE or pid != application.pid or attributes.map_state != 2:
                    continue
                require(720 <= attributes.width <= 1440 and 680 <= attributes.height <= 1000, "mapped GUI dimensions outside boundary")
                generation = process_identity(pid, unit, expected)
                screenshot = workspace / "linux-deb-gui.png"
                if screenshot.exists():
                    screenshot.unlink()
                subprocess.run([IMPORT, "-window", str(window), str(screenshot)], check=True, timeout=10)
                os.chmod(screenshot, 0o600)
                require(0 < screenshot.stat().st_size <= 8 * 1024 * 1024, "GUI screenshot size outside boundary")
                fields = subprocess.check_output([IDENTIFY, "-format", "%m %w %h %k %[fx:standard_deviation]", str(screenshot)], timeout=10).decode("ascii").split()
                require(len(fields) == 5 and fields[0] == "PNG", "GUI screenshot is not PNG")
                width, height, colors = map(int, fields[1:4])
                deviation = float(fields[4])
                if colors < 32 or deviation < 0.02:
                    break  # A mapped solid frame is not rendered GUI proof. Wait for WebKit.
                require((width, height) == (attributes.width, attributes.height), "GUI screenshot geometry differs from mapped window")
                require(process_identity(pid, unit, expected) == generation, "GUI generation changed during screenshot")
                title_after, pid_after, attributes_after = x.window(window)
                require(title_after == TITLE and pid_after == pid and attributes_after.map_state == 2 and (attributes_after.width, attributes_after.height) == (width, height), "mapped GUI changed during screenshot")
                require(digest_file(artifact) == public_digest and digest_file(GUI) == expected, "GUI/public artifact bytes changed")
                receipt = {"schema_version": 1, "proof_scope": "packaged_linux_gui_window", "artifact_sha256": public_digest,
                           "gui_sha256": expected, "uid": os.getuid(), "gid": os.getgid(), "unit": unit,
                           "pid": pid, "start_time_ticks": generation, "window_id": window, "title": TITLE,
                           "mapped": True, "width": width, "height": height, "rendered_frame": True,
                           "color_count": colors, "standard_deviation": deviation, "screenshot_sha256": digest_file(screenshot)}
                with (workspace / "linux-deb-gui-observation.json").open("x") as stream:
                    json.dump(receipt, stream, indent=2)
                    stream.write("\n")
                return
            time.sleep(0.25)
        raise RuntimeError("packaged GUI did not produce a mapped nonblank production window")
    finally:
        x.close()
        # Children can detach. Only the enclosing transient unit can establish settlement.
        if application.poll() is None:
            application.terminate()
            try:
                application.wait(timeout=5)
            except subprocess.TimeoutExpired:
                application.kill()
                application.wait(timeout=5)


def session(workspace, artifact, unit):
    directories = {name: workspace / name for name in ("home", "data", "config", "cache", "runtime", "tmp")}
    for directory in directories.values():
        directory.mkdir(mode=0o700)
    authority = workspace / "Xauthority"
    # FamilyWild matches the display chosen by -displayfd, without allowing unauthenticated clients.
    record = struct.pack(">H", 65535)
    for field in (b"", b"", b"MIT-MAGIC-COOKIE-1", os.urandom(16)):
        record += struct.pack(">H", len(field)) + field
    with authority.open("xb") as stream:
        stream.write(record)
    read_fd, write_fd = os.pipe()
    environment = {**BASE_ENV, "HOME": str(directories["home"]), "XDG_DATA_HOME": str(directories["data"]),
                   "XDG_CONFIG_HOME": str(directories["config"]), "XDG_CACHE_HOME": str(directories["cache"]),
                   "XDG_RUNTIME_DIR": str(directories["runtime"]), "TMPDIR": str(directories["tmp"]), "XAUTHORITY": str(authority)}
    server = subprocess.Popen(["/usr/bin/Xvfb", "-displayfd", str(write_fd), "-screen", "0", "1440x1000x24", "-nolisten", "tcp", "-auth", str(authority)], pass_fds=(write_fd,), env=environment)
    os.close(write_fd)
    try:
        require(select.select([read_fd], [], [], 10)[0], "private Xvfb did not report a display")
        display = os.read(read_fd, 32).decode("ascii").strip()
        require(display.isdecimal() and 0 <= int(display) <= 65535, "private display ID invalid")
        environment["DISPLAY"] = ":" + display
        subprocess.run(["/usr/bin/dbus-run-session", "--", PYTHON, "-I", str(Path(__file__).resolve()), "observe", str(workspace), str(artifact), unit], cwd=workspace, env=environment, check=True, timeout=85)
    finally:
        os.close(read_fd)
        server.terminate()
        try:
            server.wait(timeout=5)
        except subprocess.TimeoutExpired:
            server.kill()
            server.wait(timeout=5)
        if "DISPLAY" in environment:
            display = environment["DISPLAY"][1:]
            require(not Path("/tmp/.X" + display + "-lock").exists() and not Path("/tmp/.X11-unix/X" + display).exists(), "owned X display residue remains")


if __name__ == "__main__":
    os.umask(0o077)
    require(len(sys.argv) == 5 and sys.argv[1] in ("session", "observe"), "fixed GUI invocation required")
    workspace, artifact, unit = Path(sys.argv[2]), Path(sys.argv[3]), sys.argv[4]
    require(os.getuid() != 0 and os.getgid() != 0, "GUI capture requires a standard user")
    require(workspace.resolve() == workspace and workspace.stat().st_uid == os.getuid() and stat.S_IMODE(workspace.stat().st_mode) == 0o700, "GUI workspace is not private current-user directory")
    require(artifact.resolve() == artifact and not artifact.is_symlink() and artifact.parent == workspace.parent, "GUI artifact is outside owned workspace")
    require(re.fullmatch(r"batcave-deb-gui-[0-9a-f]{24}\.service", unit), "fixed GUI unit required")
    require(any(line.split(":", 2)[-1].split("/")[-1] == unit for line in Path("/proc/self/cgroup").read_text().splitlines()), "observer escaped owned unit")
    for tool in (PYTHON, "/usr/bin/Xvfb", "/usr/bin/dbus-run-session", IMPORT, IDENTIFY, "/usr/bin/dpkg-deb"):
        require(Path(tool).is_file() and not Path(tool).is_symlink() and os.access(tool, os.X_OK), "fixed GUI tool unavailable")
    (session if sys.argv[1] == "session" else observe)(workspace, artifact, unit)
