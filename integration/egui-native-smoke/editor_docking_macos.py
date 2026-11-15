"""Drive the existing eight-pane docking observer with real macOS mouse events.

Build the native basic example separately and pass its executable. Completed-pass
diagnostics locate input; the application verifies committed layout and content.
The current process needs Accessibility permission. This finite driver owns only
its editor processes and restores the pointer after releasing any held button.
"""

from __future__ import annotations

import ctypes
import argparse
import os
from pathlib import Path
import plistlib
import queue
import re
import shlex
import subprocess
import sys
import tempfile
import threading
import time


class Point(ctypes.Structure):
    _fields_ = [("x", ctypes.c_double), ("y", ctypes.c_double)]


class Size(ctypes.Structure):
    _fields_ = [("width", ctypes.c_double), ("height", ctypes.c_double)]


class Rect(ctypes.Structure):
    _fields_ = [("origin", Point), ("size", Size)]


class Mouse:
    def __init__(self) -> None:
        self.cg = ctypes.CDLL(
            "/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics"
        )
        self.cf = ctypes.CDLL(
            "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation"
        )
        for name, arguments, result in [
            ("CGPreflightPostEventAccess", [], ctypes.c_bool),
            ("CGEventCreate", [ctypes.c_void_p], ctypes.c_void_p),
            ("CGEventGetLocation", [ctypes.c_void_p], Point),
            ("CGEventCreateMouseEvent",
             [ctypes.c_void_p, ctypes.c_uint32, Point, ctypes.c_uint32],
             ctypes.c_void_p),
            ("CGEventCreateKeyboardEvent",
             [ctypes.c_void_p, ctypes.c_uint16, ctypes.c_bool], ctypes.c_void_p),
            ("CGEventPost", [ctypes.c_uint32, ctypes.c_void_p], None),
            ("CGMainDisplayID", [], ctypes.c_uint32),
            ("CGDisplayBounds", [ctypes.c_uint32], Rect),
            ("CGSessionCopyCurrentDictionary", [], ctypes.c_void_p),
            ("CGWindowListCopyWindowInfo", [ctypes.c_uint32, ctypes.c_uint32], ctypes.c_void_p),
        ]:
            function = getattr(self.cg, name)
            function.argtypes = arguments
            function.restype = result
        self.cf.CFRelease.argtypes = [ctypes.c_void_p]
        self.cf.CFRelease.restype = None
        self.cf.CFPropertyListCreateData.argtypes = [
            ctypes.c_void_p, ctypes.c_void_p, ctypes.c_int64, ctypes.c_ulong, ctypes.c_void_p,
        ]
        self.cf.CFPropertyListCreateData.restype = ctypes.c_void_p
        self.cf.CFDataGetLength.argtypes = [ctypes.c_void_p]
        self.cf.CFDataGetLength.restype = ctypes.c_long
        self.cf.CFDataGetBytePtr.argtypes = [ctypes.c_void_p]
        self.cf.CFDataGetBytePtr.restype = ctypes.c_void_p
        self.cf.CFRunLoopRunInMode.argtypes = [
            ctypes.c_void_p, ctypes.c_double, ctypes.c_bool,
        ]
        self.cf.CFRunLoopRunInMode.restype = ctypes.c_int32
        self.default_run_loop_mode = ctypes.c_void_p.in_dll(
            self.cf, "kCFRunLoopDefaultMode",
        )
        session = self.cg.CGSessionCopyCurrentDictionary()
        if not session:
            raise RuntimeError("physical docking requires an interactive macOS desktop session")
        data = None
        try:
            data = self.cf.CFPropertyListCreateData(None, session, 200, 0, None)
            if not data:
                raise RuntimeError("could not observe the macOS desktop session")
            properties = plistlib.loads(ctypes.string_at(
                self.cf.CFDataGetBytePtr(data), self.cf.CFDataGetLength(data),
            ))
            if properties.get("CGSSessionScreenIsLocked", False):
                raise RuntimeError("unlock the macOS desktop before running physical docking input")
        finally:
            if data:
                self.cf.CFRelease(data)
            self.cf.CFRelease(session)
        if not self.cg.CGPreflightPostEventAccess():
            raise RuntimeError("enable Accessibility permission for the input driver")
        event = self.cg.CGEventCreate(None)
        if not event:
            raise RuntimeError("could not read the current macOS pointer location")
        try:
            self.original = self.cg.CGEventGetLocation(event)
        finally:
            self.cf.CFRelease(event)
        self.held = False
        self.last = self.original

    def application(self, pid: int) -> tuple:
        ctypes.CDLL("/System/Library/Frameworks/AppKit.framework/AppKit")
        objc = ctypes.CDLL("/usr/lib/libobjc.A.dylib")
        objc.objc_getClass.argtypes = [ctypes.c_char_p]
        objc.objc_getClass.restype = ctypes.c_void_p
        objc.sel_registerName.argtypes = [ctypes.c_char_p]
        objc.sel_registerName.restype = ctypes.c_void_p
        receiver = objc.objc_getClass(b"NSRunningApplication")
        selector = objc.sel_registerName
        application = ctypes.CFUNCTYPE(
            ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_int,
        )(("objc_msgSend", objc))(
            receiver, selector(b"runningApplicationWithProcessIdentifier:"), pid,
        )
        if not application:
            raise RuntimeError(f"the owned editor process {pid} is unavailable")
        return objc, selector, application

    def activate(self, pid: int) -> None:
        objc, selector, application = self.application(pid)
        active = ctypes.CFUNCTYPE(
            ctypes.c_bool, ctypes.c_void_p, ctypes.c_void_p,
        )(("objc_msgSend", objc))
        self._set_application_bool(pid, b"AXFrontmost", True)
        deadline = time.monotonic() + 3
        while not active(application, selector(b"isActive")):
            if time.monotonic() >= deadline:
                raise RuntimeError(f"the owned editor process {pid} did not become active")
            self.cf.CFRunLoopRunInMode(self.default_run_loop_mode, 0.02, False)

    def set_hidden(self, pid: int, hidden: bool) -> None:
        objc, selector, application = self.application(pid)
        send = ctypes.CFUNCTYPE(
            ctypes.c_bool, ctypes.c_void_p, ctypes.c_void_p,
        )(("objc_msgSend", objc))
        self._set_application_bool(pid, b"AXHidden", hidden)
        deadline = time.monotonic() + 3
        while (send(application, selector(b"isHidden")) != hidden
               or (hidden and send(application, selector(b"isActive")))):
            if time.monotonic() >= deadline:
                expected = "hidden and inactive" if hidden else "visible"
                raise RuntimeError(f"the owned editor did not become {expected}")
            self.cf.CFRunLoopRunInMode(self.default_run_loop_mode, 0.02, False)

    def _set_application_bool(self, pid: int, name: bytes, value: bool) -> None:
        # NSRunningApplication control can reject the unbundled editor. Use its
        # real accessibility interface, retaining native focus/capture callbacks.
        services = ctypes.CDLL(
            "/System/Library/Frameworks/ApplicationServices.framework/ApplicationServices"
        )
        services.AXUIElementCreateApplication.argtypes = [ctypes.c_int]
        services.AXUIElementCreateApplication.restype = ctypes.c_void_p
        services.AXUIElementSetAttributeValue.argtypes = [
            ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p,
        ]
        services.AXUIElementSetAttributeValue.restype = ctypes.c_int32
        self.cf.CFStringCreateWithCString.argtypes = [
            ctypes.c_void_p, ctypes.c_char_p, ctypes.c_uint32,
        ]
        self.cf.CFStringCreateWithCString.restype = ctypes.c_void_p
        element = services.AXUIElementCreateApplication(pid)
        attribute = self.cf.CFStringCreateWithCString(None, name, 0x08000100)
        try:
            if not element or not attribute:
                raise RuntimeError("could not create the owned editor's accessibility target")
            boolean = "kCFBooleanTrue" if value else "kCFBooleanFalse"
            status = services.AXUIElementSetAttributeValue(
                element, attribute, ctypes.c_void_p.in_dll(self.cf, boolean),
            )
            if status:
                raise RuntimeError(
                    f"could not set {name!r}={value} for the owned editor: status={status}"
                )
        finally:
            if attribute:
                self.cf.CFRelease(attribute)
            if element:
                self.cf.CFRelease(element)

    def escape(self) -> None:
        for pressed in [True, False]:
            event = self.cg.CGEventCreateKeyboardEvent(None, 53, pressed)
            if not event:
                raise RuntimeError("could not create a native Escape event")
            try:
                self.cg.CGEventPost(0, event)
            finally:
                self.cf.CFRelease(event)

    def windows(self, pid: int) -> list[dict]:
        info = self.cg.CGWindowListCopyWindowInfo(0, 0)
        if not info:
            raise RuntimeError("could not observe native OS window inventory")
        data = None
        try:
            data = self.cf.CFPropertyListCreateData(None, info, 200, 0, None)
            if not data:
                raise RuntimeError("could not decode native OS window inventory")
            raw = ctypes.string_at(
                self.cf.CFDataGetBytePtr(data), self.cf.CFDataGetLength(data),
            )
            return [window for window in plistlib.loads(raw)
                    if window.get("kCGWindowOwnerPID") == pid
                    and window.get("kCGWindowLayer") == 0]
        finally:
            if data:
                self.cf.CFRelease(data)
            self.cf.CFRelease(info)

    def window_at_frame(self, pid: int, frame: tuple[float, ...]) -> int:
        left, top, right, bottom = frame
        matches = []
        for window in self.windows(pid):
            bounds = window["kCGWindowBounds"]
            if (abs(bounds["X"] - left) < 2
                and abs(bounds["Width"] - (right - left)) < 2
                and 0 <= top - bounds["Y"] < 64
                and abs(bounds["Y"] + bounds["Height"] - bottom) < 2):
                matches.append(window["kCGWindowNumber"])
        if len(matches) != 1:
            raise RuntimeError(f"expected one owned OS window for client geometry {frame}")
        return matches[0]

    def visible_window_at(self, pid: int, point: Point) -> int | None:
        for window in self.windows(pid):
            bounds = window["kCGWindowBounds"]
            if (window.get("kCGWindowIsOnscreen")
                and bounds["X"] <= point.x < bounds["X"] + bounds["Width"]
                and bounds["Y"] <= point.y < bounds["Y"] + bounds["Height"]):
                return window["kCGWindowNumber"]
        return None

    def close_window(self, pid: int, window_id: int) -> None:
        self.activate(pid)
        window = next(window for window in self.windows(pid)
                      if window["kCGWindowNumber"] == window_id)
        bounds = window["kCGWindowBounds"]
        # This finite profile uses the standard AppKit titled window chrome.
        point = Point(bounds["X"] + 13, bounds["Y"] + 12)
        if self.visible_window_at(pid, point) != window_id:
            raise RuntimeError("the owned Child's native close button is obscured")
        self.move(point)
        time.sleep(0.12)
        self.press()
        self.release()

    def post(self, kind: int, point: Point) -> None:
        event = self.cg.CGEventCreateMouseEvent(None, kind, point, 0)
        if not event:
            raise RuntimeError("could not create a macOS mouse event")
        try:
            self.cg.CGEventPost(0, event)
            self.last = point
        finally:
            self.cf.CFRelease(event)

    def move(self, point: Point) -> None:
        self.post(6 if self.held else 5, point)  # leftMouseDragged / mouseMoved

    def press(self) -> None:
        self.post(1, self.last)
        self.held = True

    def release(self) -> None:
        if self.held:
            self.post(2, self.last)
            self.held = False

    def restore(self) -> None:
        self.release()
        self.move(self.original)

    def leave_and_return(self, frame: tuple[float, ...], point: Point) -> None:
        left, top, right, bottom = frame
        display = self.cg.CGDisplayBounds(self.cg.CGMainDisplayID())
        for x, y in [
            (display.origin.x + 80, display.origin.y + display.size.height - 80),
            (display.origin.x + display.size.width - 80, display.origin.y + 80),
        ]:
            if not (left <= x < right and top <= y < bottom):
                self.move(Point(x, y))
                time.sleep(0.15)
                self.move(point)
                time.sleep(0.15)
                return
        raise RuntimeError("the desktop needs space outside the editor client area")


def coordinates(log: Path, viewport: str = "FFFF") -> tuple[float, tuple[float, ...]]:
    latest = None
    for line in log.read_text().splitlines():
        if f'\tviewport\tid="{viewport}"' not in line:
            continue
        zoom = re.search(r"\bzoom=([\d.]+)", line)
        scale = re.search(r"native_pixels_per_point: Some\(([\d.]+)\)", line)
        bounds = re.search(
            r"inner_rect: Some\(\[\[([-\d.]+) ([-\d.]+)\] - "
            r"\[([-\d.]+) ([-\d.]+)\]\]\)", line,
        )
        if zoom and scale and bounds:
            latest = (
                float(zoom[1]), float(scale[1]), tuple(map(float, bounds.groups())),
            )
    if latest is None or latest[1] <= 0:
        raise RuntimeError("the editor has not supplied its real client geometry")
    if latest[0] != 1.0:
        raise RuntimeError("this finite docking profile requires egui zoom 1.0")
    return latest[1], latest[2]


def run_case(binary: Path, case: str, artifacts: Path, mouse: Mouse) -> None:
    log = artifacts / f"{case}-events.log"
    process = subprocess.Popen(
        [str(binary), "--docking-smoke", "--event-log", str(log)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        text=True, bufsize=1,
        env=dict(os.environ, EGUI_DOCKSPACE_DOCKING_CASE=case),
    )
    assert process.stdout is not None
    lines: queue.Queue[str | None] = queue.Queue()

    def read_output() -> None:
        with (artifacts / f"{case}-console.log").open("w") as output:
            for line in process.stdout:
                output.write(line)
                output.flush()
                lines.put(line)
        lines.put(None)

    reader = threading.Thread(target=read_output, daemon=True)
    reader.start()
    completed = False
    failed = False
    eof_seen = False
    drags = 0
    native_windows: dict[str, int] = {}
    native_retired = False
    native_target: Point | None = None
    interrupt_case = case.startswith("native-cancel-")
    interrupt_verified = False
    interrupt_tail = False
    try:
        deadline = time.monotonic() + (45 if case.startswith(("drain-", "native-")) else 20)
        while time.monotonic() < deadline:
            try:
                line = lines.get(timeout=1)
            except queue.Empty:
                continue
            if line is None:
                eof_seen = True
                break
            print(f"{case}: {line}", end="", flush=True)
            failed |= "Native dockspace stopped" in line or "panicked at" in line
            if line.startswith("editor-native-drag:"):
                arguments = shlex.split(line.split(":", 1)[1])

                def native_point(offset: int) -> tuple[Point, tuple[float, ...]]:
                    viewport, _, x, y = arguments[offset:offset + 4]
                    scale, frame = coordinates(log, viewport)
                    if scale != 1.0:
                        raise RuntimeError("the native-window docking profile requires native scale 1.0")
                    actual = mouse.window_at_frame(process.pid, frame)
                    previous = native_windows.setdefault(viewport, actual)
                    if actual != previous:
                        raise RuntimeError("a physical docking step replaced its original OS window")
                    point = Point(frame[0] + float(x) / scale, frame[1] + float(y) / scale)
                    if mouse.visible_window_at(process.pid, point) != actual:
                        raise RuntimeError("the native docking point is covered by another owned window")
                    return point, frame

                mouse.activate(process.pid)
                source, source_frame = native_point(0)
                mouse.move(source)
                time.sleep(0.12)
                mouse.press()
                target, target_frame = native_point(4)
                time.sleep(0.12)
                mouse.move(Point(source.x + 24, source.y + 4))
                time.sleep(0.12)
                if interrupt_case and not interrupt_verified:
                    mouse.move(target)
                    native_target = target
                    continue
                if drags == 0:
                    display = mouse.cg.CGDisplayBounds(mouse.cg.CGMainDisplayID())
                    outside = Point(display.origin.x + 50, display.origin.y + display.size.height - 50)
                    if (outside.y <= source_frame[3]
                        or any(l <= outside.x < r and t <= outside.y < b
                               for l, t, r, b in [source_frame, target_frame])):
                        raise RuntimeError("the native profile needs desktop space below the source and outside both windows")
                    mouse.move(outside)
                    native_target = target
                    continue
                mouse.move(Point(target.x - 25, target.y))
                time.sleep(0.12)
                mouse.move(target)
                time.sleep(0.2)
                mouse.release()
                drags += 1
            elif line.startswith("editor-native-interrupt:"):
                if not interrupt_case or not mouse.held or native_target is None:
                    raise RuntimeError("the interrupt must follow a held native drop preview")
                action = line.split(":", 1)[1].split()[0]
                if action == "escape" and case == "native-cancel-escape":
                    mouse.escape()
                elif action == "deactivate" and case == "native-cancel-deactivate":
                    mouse.set_hidden(process.pid, True)
                    # Wait while the real applicationDidResignActive callback runs.
                    time.sleep(0.2)
                    mouse.set_hidden(process.pid, False)
                    mouse.activate(process.pid)
                else:
                    raise RuntimeError("the interruption does not match the selected case")
            elif line.startswith("editor-native-interrupt-cancelled:"):
                if native_target is None or not mouse.held or interrupt_tail:
                    raise RuntimeError("the cancellation must precede the original release")
                mouse.move(Point(native_target.x - 25, native_target.y))
                time.sleep(0.12)
                mouse.move(native_target)
                time.sleep(0.12)
                mouse.release()
                assert process.stdin is not None
                process.stdin.write("native-interrupt-tail-sent\n")
                process.stdin.flush()
                native_target = None
                interrupt_tail = True
            elif line.startswith("editor-native-interrupt-verified:"):
                if not interrupt_tail or interrupt_verified:
                    raise RuntimeError("native interruption verification is out of order")
                interrupt_verified = True
            elif line.startswith("editor-native-outside-observed:"):
                if native_target is None or not mouse.held or drags:
                    raise RuntimeError("held outside receipt does not match the initial native drag")
                mouse.move(Point(native_target.x - 25, native_target.y))
                time.sleep(0.12)
                mouse.move(native_target)
                time.sleep(0.2)
                mouse.release()
                drags += 1
                native_target = None
            elif line.startswith("editor-native-empty-central:"):
                viewport, _ = shlex.split(line.split(":", 1)[1])
                child = native_windows[viewport]
                if (case not in ("native-inspector-first", "native-cancel-escape", "native-cancel-deactivate")
                        or drags != 4):
                    raise RuntimeError("only the drained central Child may request native close")
                current = {window["kCGWindowNumber"] for window in mouse.windows(process.pid)}
                if child not in current or native_windows["FFFF"] not in current:
                    raise RuntimeError("the empty central Child and original Root must remain alive")
                mouse.close_window(process.pid, child)
            elif line.startswith("editor-native-retired:"):
                root = native_windows.get("FFFF")
                children = set(native_windows.values()) - {root}
                if root is None or len(children) != 1:
                    raise RuntimeError("native retirement requires the original Root and one Child")
                retirement_deadline = time.monotonic() + 3
                while True:
                    current = {window["kCGWindowNumber"] for window in mouse.windows(process.pid)}
                    if root not in current:
                        raise RuntimeError("native retirement destroyed the original Root")
                    if not children & current:
                        break
                    if time.monotonic() >= retirement_deadline:
                        raise RuntimeError("the emptied Child's OS window did not retire")
                    time.sleep(0.02)
                native_retired = True
                assert process.stdin is not None
                process.stdin.write("native-retirement-observed\n")
                process.stdin.flush()
            elif line.startswith((
                "editor-docking-select-target:",
                "editor-docking-ready:", "editor-docking-resize-ready:",
            )):
                _, *arguments = line.split(":", 1)[1].split()
                values = list(map(float, arguments))
                scale, frame = coordinates(log)
                mouse.activate(process.pid)

                def point(x: float, y: float) -> Point:
                    return Point(frame[0] + x / scale, frame[1] + y / scale)

                if line.startswith("editor-docking-select-target:"):
                    mouse.move(point(*values))
                    time.sleep(0.12)
                    mouse.press()
                    mouse.release()
                    continue
                sx, sy, tx, ty = values
                is_tab = line.startswith("editor-docking-ready:")
                reenter = is_tab and drags == 0 and case.startswith("drain-")
                drags += int(is_tab)
                mouse.move(point(sx, sy))
                time.sleep(0.12)
                mouse.press()
                time.sleep(0.12)
                for step in range(1, 13):
                    current = point(
                        sx + (tx - sx) * step / 12,
                        sy + (ty - sy) * step / 12,
                    )
                    mouse.move(current)
                    time.sleep(0.025)
                    if reenter and step == 4:
                        mouse.leave_and_return(frame, current)
                time.sleep(0.2)
                mouse.release()
            elif line.startswith("editor-docking-complete:"):
                completed = True
        if not eof_seen:
            raise RuntimeError(f"editor docking timed out for {case}; artifacts={artifacts}")
        status = process.wait(timeout=5)
        failed |= "\tfatal\t" in log.read_text()
        if case.startswith("native-") and (drags != 4 or not native_retired):
            failed = True
        if interrupt_case and not interrupt_verified:
            failed = True
        if status or not completed or failed:
            raise RuntimeError(
                f"editor docking failed for {case}: exit={status}, "
                f"completed={completed}, runtime_failed={failed}; "
                f"artifacts={artifacts}"
            )
    finally:
        try:
            mouse.release()
        finally:
            try:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=5)
            finally:
                reader.join(timeout=3)
                process.stdout.close()
                if process.stdin is not None:
                    process.stdin.close()


def main() -> None:
    if sys.platform != "darwin":
        raise SystemExit("this driver requires macOS")
    cases = [
        "outline", "console", "inspector", "preview", "profiler",
        "drain-inspector-first", "drain-preview-first",
        "native-inspector-first", "native-preview-first",
        "native-cancel-escape", "native-cancel-deactivate",
    ]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--case", choices=cases, action="append", dest="cases")
    arguments = parser.parse_args()
    binary = arguments.binary.resolve(strict=True)
    mouse = Mouse()
    artifacts = Path(tempfile.mkdtemp(prefix="dockspace-macos-docking-"))
    print(f"macOS docking artifacts: {artifacts}", flush=True)
    try:
        for case in arguments.cases or cases:
            run_case(binary, case, artifacts, mouse)
    finally:
        mouse.restore()
    print(f"editor-macos: PASS; {len(arguments.cases or cases)} real-input journeys", flush=True)


if __name__ == "__main__":
    main()
