"""Drive native basic's seven contained-docking journeys through real Wayland input.

The compositor supplies geometry only to this driver. The application observes
ordinary seat input and verifies the same committed merges, contained retirement,
held leave/reentry, and two-axis splitter changes used by the X11/macOS journeys.
"""

from __future__ import annotations

import argparse
import json
import math
import os
from pathlib import Path
import queue
import subprocess
import tempfile
import threading
import time

from editor_docking_wayland import WaylandPointer, nodes, query


CASES = (
    "outline", "console", "inspector", "preview", "profiler",
    "drain-inspector-first", "drain-preview-first",
)


def leave_and_return(pointer: WaylandPointer, returning: tuple[float, float]) -> None:
    window = pointer.window()
    rect, client = window["rect"], window["window_rect"]
    left, top = rect["x"] + client["x"], rect["y"] + client["y"]
    right, bottom = left + client["width"], top + client["height"]
    outputs = [output for output in query("get_outputs") if output.get("active")]
    if len(outputs) != 1:
        raise RuntimeError("the native Wayland profile requires one owned headless output")
    extent = outputs[0]["rect"]
    if extent["x"] or extent["y"]:
        raise RuntimeError("the native Wayland profile requires output origin 0,0")
    width, height = extent["width"], extent["height"]
    for x, y in ((10, height - 10), (width - 10, height - 10), (10, 10)):
        if not (left <= x < right and top <= y < bottom):
            pointer.send(f"move {x} {y} {width} {height}")
            time.sleep(0.2)
            pointer.move(*returning)
            time.sleep(0.2)
            return
    raise RuntimeError("the compositor needs desktop space outside the editor client")


def run_case(binary: Path, case: str, artifacts: Path) -> None:
    directory = artifacts / case
    directory.mkdir()
    event_log = directory / "operations.log"
    process = subprocess.Popen(
        [str(binary), "--docking-smoke", "--event-log", str(event_log)],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1,
        env=os.environ | {
            "EGUI_DOCKSPACE_DOCKING_CASE": case,
            "EGUI_DOCKSPACE_WAYLAND_INPUT": "1",
        },
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
    held = completed = failed = eof_seen = False
    pending: tuple[str, tuple[float, float], tuple[float, float]] | None = None
    stage = ""
    tab_drags = merges = 0
    resized: list[str] = []
    left_and_returned = False
    target_entered_at: float | None = None
    stuck_captured = False
    try:
        pointer = WaylandPointer()
        with (directory / "stdout.log").open("x") as log:
            deadline = time.monotonic() + (50 if case.startswith("drain-") else 25)
            while time.monotonic() < deadline:
                try:
                    line = lines.get(timeout=0.5)
                except queue.Empty:
                    if pending is not None and stage == "hover" and not held:
                        pointer.move(*pending[1])
                    elif (
                        pending is not None
                        and stage == "target"
                        and held
                        and not stuck_captured
                        and target_entered_at is not None
                        and time.monotonic() - target_entered_at >= 3.0
                    ):
                        subprocess.run(
                            ["grim", str(directory / "target-stuck.png")],
                            check=False,
                            timeout=3,
                        )
                        stuck_captured = True
                    continue
                if line is None:
                    eof_seen = True
                    break
                print(f"{case}: {line}", end="", flush=True)
                log.write(line)
                log.flush()
                failed |= "Native dockspace stopped" in line or "panicked at" in line
                values = line.split(":", 1)[-1].split()
                if line.startswith("editor-wayland-scale:"):
                    scale = float(values[0])
                    if not math.isfinite(scale) or scale <= 0:
                        raise RuntimeError("the application reported an invalid client scale")
                    pointer.client_scale = scale
                elif line.startswith((
                    "editor-docking-select-target:", "editor-docking-ready:",
                    "editor-docking-resize-ready:",
                )):
                    if pending is not None or held:
                        raise RuntimeError("a new gesture replaced the current real press")
                    # The numeric token belongs to winit, not an X11 window.
                    # Resolve only this process's exact xdg-shell compositor ID.
                    pointer.activate(process.pid)
                    (directory / "window.json").write_text(json.dumps(pointer.window(), indent=2))
                    if line.startswith("editor-docking-select-target:"):
                        _, x, y = values
                        source = target = (float(x), float(y))
                        kind = "select"
                    else:
                        _, sx, sy, tx, ty = values
                        source, target = (float(sx), float(sy)), (float(tx), float(ty))
                        kind = "tab" if line.startswith("editor-docking-ready:") else "resize"
                    pending = kind, source, target
                    stage = "hover"
                    pointer.move(*source)
                elif line.startswith("editor-wayland-press:"):
                    if pending is None or stage != "hover" or held:
                        raise RuntimeError("the original source hover must precede a real press")
                    pointer.press()
                    held = True
                    stage = "press"
                elif line.startswith("editor-wayland-drag:"):
                    if pending is None or stage != "press" or not held:
                        raise RuntimeError("the application must observe the original held press")
                    kind, source, target = pending
                    if kind == "select":
                        pointer.window()
                        pointer.release()
                        held = False
                        pending = None
                        stage = ""
                        continue
                    for step in range(1, 13):
                        current = tuple(a + (b - a) * step / 12 for a, b in zip(source, target))
                        pointer.move(*current)
                        time.sleep(0.025)
                        if kind == "tab" and tab_drags == 0 and case.startswith("drain-") and step == 4:
                            leave_and_return(pointer, current)
                            left_and_returned = True
                    if kind == "resize":
                        pointer.window()
                        pointer.release()
                        held = False
                        pending = None
                        stage = ""
                    else:
                        stage = "target"
                        target_entered_at = time.monotonic()
                elif line.startswith("editor-wayland-target:"):
                    if pending is None or pending[0] != "tab" or stage != "target" or not held:
                        raise RuntimeError("the actual center guide must belong to the held tab")
                    pointer.move(*map(float, values))
                    time.sleep(0.2)
                    subprocess.run(
                        ["grim", str(directory / f"preview-{tab_drags}.png")], check=True, timeout=3,
                    )
                    pointer.window()
                    pointer.release()
                    held = False
                    tab_drags += 1
                    pending = None
                    stage = ""
                elif line.startswith("editor-docking-merged:"):
                    merges += 1
                elif line.startswith("editor-docking-resized:"):
                    resized.append(values[0].rstrip(";"))
                elif line.startswith("editor-docking-complete:"):
                    completed = True
            if not eof_seen:
                subprocess.run(
                    ["grim", str(directory / "failure.png")], check=False, timeout=3,
                )
                raise RuntimeError(f"native Wayland docking timed out for {case}")
            status = process.wait(timeout=3)
            failed |= "\tfatal\t" in event_log.read_text()
            drain = case.startswith("drain-")
            if (status or not completed or failed or held or pending is not None
                    or tab_drags != (2 if drain else 1) or merges != tab_drags
                    or resized != (["Horizontal", "Vertical"] if drain else [])
                    or left_and_returned != drain):
                raise RuntimeError(
                    f"native Wayland {case} failed: exit={status}, complete={completed}, "
                    f"fatal={failed}, tab_drags={tab_drags}, merges={merges}, resized={resized}, "
                    f"held={held}, pending={pending}, leave_reentry={left_and_returned}",
                )
            retirement = time.monotonic() + 3
            while any(node.get("pid") == process.pid and node.get("shell")
                      for node in nodes(query("get_tree"))):
                if time.monotonic() >= retirement:
                    raise RuntimeError("the exact owned Wayland toplevel did not retire after exit")
                time.sleep(0.02)
    finally:
        try:
            if held and pointer is not None:
                pointer.release()
        finally:
            try:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=3)
            finally:
                reader.join(timeout=3)
                process.stdout.close()
                if pointer is not None:
                    pointer.close()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--artifacts", type=Path)
    parser.add_argument("--case", choices=CASES, action="append", dest="cases")
    args = parser.parse_args()
    if os.environ.get("DISPLAY") or not os.environ.get("WAYLAND_DISPLAY") or not os.environ.get("SWAYSOCK"):
        parser.error("use with_sway.py in its isolated native Wayland session without DISPLAY")
    binary = args.binary.resolve(strict=True)
    artifacts = args.artifacts or Path(tempfile.mkdtemp(prefix="dockspace-native-wayland-"))
    artifacts.mkdir(parents=True, exist_ok=True)
    (artifacts / "compositor.json").write_text(json.dumps(query("get_version"), indent=2))
    (artifacts / "outputs.json").write_text(json.dumps(query("get_outputs"), indent=2))
    print(f"native Wayland docking artifacts: {artifacts.resolve()}", flush=True)
    cases = args.cases or CASES
    for case in cases:
        run_case(binary, case, artifacts)
    print(f"native-editor-wayland: PASS; {len(cases)} real xdg-shell input journeys", flush=True)


if __name__ == "__main__":
    main()
