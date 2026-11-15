"""Qualify the actual editor through an isolated Sway compositor's seat input.

Sway IPC geometry belongs exclusively to this test driver. The application sees
ordinary Wayland input and retains its conservative backend capabilities.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import queue
import select
import subprocess
import tempfile
import threading
import time

from with_sway import sway


def query(kind: str) -> object:
    return json.loads(subprocess.check_output(
        ["swaymsg", "--raw", "--type", kind], text=True, timeout=3,
    ))


def nodes(node: dict):
    yield node
    for child in node.get("nodes", []) + node.get("floating_nodes", []):
        yield from nodes(child)


class WaylandPointer:
    def __init__(self) -> None:
        self.pid: int | None = None
        self.container: int | None = None
        self.client_scale = 1.0
        self.device = subprocess.Popen(
            [os.environ["DOCKSPACE_WAYLAND_SEAT_HELPER"]], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, text=True, bufsize=1,
        )
        assert self.device.stdout is not None
        ready = select.select([self.device.stdout], [], [], 3)[0]
        if not ready or self.device.stdout.readline().strip() != "wayland-seat: virtual pointer ready":
            self.close()
            raise RuntimeError("the virtual pointer did not become ready")

    def send(self, command: str) -> None:
        assert self.device.stdin is not None
        self.device.stdin.write(command + "\n")
        self.device.stdin.flush()

    def close(self) -> None:
        if self.device.stdin is not None:
            try:
                self.device.stdin.close()
            except BrokenPipeError:
                pass
        try:
            self.device.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.device.kill()
            self.device.wait(timeout=3)
        if self.device.stdout is not None:
            self.device.stdout.close()

    def window(self) -> dict:
        windows = [node for node in nodes(query("get_tree"))
                   if node.get("pid") == self.pid and node.get("shell")]
        if len(windows) != 1:
            raise RuntimeError("the finite Wayland profile requires one exact owned toplevel")
        window = windows[0]
        if window.get("shell") != "xdg_shell" or window.get("window") is not None:
            raise RuntimeError("the editor is not a native Wayland xdg-shell client")
        if self.container is not None and window["id"] != self.container:
            raise RuntimeError("the editor replaced its original compositor toplevel")
        self.container = window["id"]
        return window

    def activate(self, pid: int) -> None:
        if self.pid is not None and self.pid != pid:
            raise RuntimeError("a pointer driver cannot switch editor ownership")
        self.pid = pid
        deadline = time.monotonic() + 3
        while True:
            try:
                window = self.window()
                break
            except RuntimeError:
                if self.container is not None or time.monotonic() >= deadline:
                    raise
                time.sleep(0.05)
        sway(f'[con_id={window["id"]}] focus', os.environ)

    def move(self, x: float, y: float) -> None:
        window = self.window()
        outputs = [output for output in query("get_outputs") if output.get("active")]
        if len(outputs) != 1:
            raise RuntimeError("the finite Wayland profile requires one headless output")
        output = outputs[0]
        rect, client = window["rect"], window["window_rect"]
        sx = round(rect["x"] + client["x"] + x / self.client_scale)
        sy = round(rect["y"] + client["y"] + y / self.client_scale)
        extent = output["rect"]
        if extent["x"] != 0 or extent["y"] != 0:
            raise RuntimeError("the finite headless profile requires output origin 0,0")
        self.send(f'move {sx} {sy} {extent["width"]} {extent["height"]}')

    def press(self) -> None:
        self.send("down")

    def release(self) -> None:
        self.send("up")

    def scroll(self) -> None:
        self.send("scroll")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--artifacts", type=Path)
    parser.add_argument("--change-scale", action="store_true")
    arguments = parser.parse_args()
    if os.environ.get("DISPLAY") or not os.environ.get("WAYLAND_DISPLAY") or not os.environ.get("SWAYSOCK"):
        parser.error("use with_sway.py with an isolated Wayland session and no DISPLAY")
    artifacts = arguments.artifacts or Path(tempfile.mkdtemp(prefix="dockspace-wayland-editor-"))
    artifacts.mkdir(parents=True, exist_ok=True)
    print(f"Wayland editor artifacts: {artifacts.resolve()}", flush=True)
    (artifacts / "compositor.json").write_text(json.dumps(query("get_version"), indent=2))
    (artifacts / "outputs.json").write_text(json.dumps(query("get_outputs"), indent=2))
    binary = arguments.binary.resolve(strict=True)
    run_workbench(binary, artifacts, change_scale=arguments.change_scale)
    print("editor-wayland: PASS; native xdg-shell, real seat input and exact workbench assertions", flush=True)


def run_workbench(binary: Path, artifacts: Path, *, change_scale: bool) -> None:
    process = subprocess.Popen(
        [str(binary), "--physical-smoke"], stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT, text=True, bufsize=1,
        env=os.environ | {"DOCKSPACE_PHYSICAL_SCALE_CHANGE": "1" if change_scale else "0"},
    )
    assert process.stdout is not None
    lines: queue.Queue[str | None] = queue.Queue()

    def read_output() -> None:
        for line in process.stdout:
            lines.put(line)
        lines.put(None)

    reader = threading.Thread(target=read_output, daemon=True)
    reader.start()
    pointer = None
    held = completed = failed = eof_seen = scale_verified = False
    actions: list[str] = []
    pending_drag: tuple[list[float], list[float]] | None = None
    try:
        pointer = WaylandPointer()
        with (artifacts / "workbench.log").open("w") as log:
            deadline = time.monotonic() + 70
            while time.monotonic() < deadline:
                try:
                    line = lines.get(timeout=1)
                except queue.Empty:
                    if pending_drag is not None and not held:
                        # A map/focus transaction can change local coordinates
                        # after pointer-enter. Keep moving to the source, but
                        # never press before the app observes its exact hover.
                        pointer.move(*pending_drag[0])
                    continue
                if line is None:
                    eof_seen = True
                    break
                print(line, end="", flush=True)
                log.write(line)
                log.flush()
                failed |= "panicked at" in line or "Native dockspace stopped" in line
                values = line.split(":", 1)[-1].split()
                if line.startswith("workbench-physical-ready:"):
                    pointer.client_scale = float(values[2])
                    pointer.activate(process.pid)
                    (artifacts / "window.json").write_text(json.dumps(pointer.window(), indent=2))
                elif line.startswith("workbench-pointer:"):
                    if pending_drag is not None or held:
                        raise RuntimeError("the previous pointer journey did not finish")
                    label, sx, sy, tx, ty = values
                    actions.append(label)
                    pointer.activate(process.pid)
                    source, target = list(map(float, (sx, sy))), list(map(float, (tx, ty)))
                    pending_drag = source, target
                    pointer.move(*source)
                elif line.startswith("workbench-press:"):
                    if pending_drag is None or held:
                        raise RuntimeError("the real source hover must precede the press")
                    pointer.press()
                    held = True
                elif line.startswith("workbench-drag:"):
                    if pending_drag is None or not held:
                        raise RuntimeError("the application must observe the original held press")
                    source, target = pending_drag
                    pending_drag = None
                    for step in range(1, 13):
                        pointer.move(*(round(a + (b - a) * step / 12)
                                          for a, b in zip(source, target)))
                        time.sleep(0.025)
                    if actions[-1] == "resize":
                        pointer.release()
                        held = False
                elif line.startswith("workbench-target:"):
                    if not held or actions[-1] not in ("preview", "inspector"):
                        raise RuntimeError("a merge target must follow the original held tab drag")
                    pointer.move(*map(float, values))
                    time.sleep(0.2)
                    subprocess.run(["grim", str(artifacts / f"{actions[-1]}-preview.png")],
                                   check=True, timeout=3)
                    pointer.release()
                    held = False
                elif line.startswith("workbench-scale-change:"):
                    if not change_scale or held:
                        raise RuntimeError("unexpected compositor scale change")
                    sway("output HEADLESS-1 mode 3200x2400 scale 2", os.environ)
                elif line.startswith("workbench-scale-verified:"):
                    outputs = query("get_outputs")
                    if not change_scale or not any(item.get("scale") == 2 for item in outputs):
                        raise RuntimeError("the compositor did not confirm the real output scale")
                    pointer.client_scale = float(values[0])
                    (artifacts / "scaled-outputs.json").write_text(json.dumps(outputs, indent=2))
                    scale_verified = True
                elif line.startswith("workbench-text:"):
                    actions.append("text")
                    # Wtype installs its own keymap. Focus the real TextEdit
                    # after that native keyboard transition, before typing.
                    keyboard = subprocess.Popen(["wtype", "-s", "500", "-d", "20",
                                                 "waylandinput", "-s", "200"])
                    try:
                        time.sleep(0.2)
                        pointer.move(*map(float, values))
                        pointer.press()
                        held = True
                        time.sleep(0.08)
                        pointer.release()
                        held = False
                        if keyboard.wait(timeout=3):
                            raise RuntimeError("the virtual keyboard could not type into the editor")
                    finally:
                        if keyboard.poll() is None:
                            keyboard.terminate()
                            keyboard.wait(timeout=3)
                elif line.startswith("workbench-scroll:"):
                    actions.append("scroll")
                    pointer.move(*map(float, values))
                    time.sleep(0.15)
                    for _ in range(4):
                        pointer.scroll()
                        time.sleep(0.025)
                    subprocess.run(["grim", str(artifacts / "final.png")], check=True, timeout=3)
                elif line.startswith("workbench-physical: PASS;"):
                    completed = True
            if not eof_seen:
                raise RuntimeError("the physical Wayland editor exceeded its deadline")
            status = process.wait(timeout=3)
            if (status or not completed or failed or held or pending_drag is not None
                    or actions != ["preview", "inspector", "resize", "text", "scroll"]
                    or scale_verified != change_scale):
                raise RuntimeError(f"Wayland workbench failed: exit={status}, completed={completed}, "
                                   f"actions={actions}, scale_verified={scale_verified}")
    finally:
        try:
            if held and pointer is not None:
                try:
                    pointer.release()
                except OSError:
                    pass
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=3)
            reader.join(timeout=3)
            process.stdout.close()
            if pointer is not None:
                pointer.close()


if __name__ == "__main__":
    main()
