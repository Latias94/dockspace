"""Run a command in an owned headless Sway session, without XWayland fallback."""

from __future__ import annotations

import argparse
import json
import math
import os
from pathlib import Path
import signal
import stat
import subprocess
import sys
import tempfile
import time


def sway(command: str, environment: dict[str, str]) -> object:
    result = subprocess.check_output(
        ["swaymsg", "--raw", command], env=environment, text=True, timeout=3,
    )
    replies = json.loads(result)
    if not replies or any(not reply.get("success") for reply in replies):
        raise RuntimeError(f"Sway rejected {command!r}: {replies}")
    return replies


def stop_owned_group(process: subprocess.Popen | None) -> None:
    if process is None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        process.wait(timeout=3)
    except subprocess.TimeoutExpired:
        pass
    finally:
        # The driver may already have exited while an owned app is still alive.
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    process.wait(timeout=3)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--timeout", type=float, default=240)
    parser.add_argument("--artifacts", type=Path)
    parser.add_argument("--seat-helper", type=Path, required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command or not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("provide a command after -- and a positive finite timeout")
    if sys.platform != "linux" or os.geteuid() == 0:
        parser.error("run on Linux as an unprivileged user; headless Sway does not need root")
    artifacts = args.artifacts or Path(tempfile.mkdtemp(prefix="dockspace-wayland-"))
    artifacts.mkdir(parents=True, exist_ok=True)
    runtime = Path(tempfile.mkdtemp(prefix="runtime-", dir=artifacts)).resolve()
    configuration = runtime / "sway.conf"
    configuration.write_text(
        "xwayland disable\n"
        "output HEADLESS-1 mode 1600x1200 scale 1\n"
        "seat seat0 fallback true\n"
        "default_border none\n"
        "default_floating_border none\n"
        "focus_follows_mouse no\n"
        'for_window [title=".*"] floating enable, resize set 1280 820, move position 60 60\n'
    )
    environment = dict(os.environ)
    for name in ("DISPLAY", "WAYLAND_DISPLAY", "SWAYSOCK"):
        environment.pop(name, None)
    environment.update(
        XDG_RUNTIME_DIR=str(runtime), XDG_SESSION_TYPE="wayland",
        WINIT_UNIX_BACKEND="wayland", WLR_BACKENDS="headless",
        WLR_RENDERER="pixman", WLR_HEADLESS_OUTPUTS="1",
        LIBGL_ALWAYS_SOFTWARE="1", LP_NUM_THREADS="1",
        DOCKSPACE_WAYLAND_SEAT_HELPER=str(args.seat_helper.resolve(strict=True)),
    )
    print(f"Wayland artifacts: {artifacts.resolve()}", flush=True)
    child = None
    keyboard = None
    with (artifacts / "sway.log").open("w") as log:
        compositor = subprocess.Popen(
            ["sway", "--unsupported-gpu", "--config", str(configuration)],
            env=environment, stdout=log, stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        try:
            deadline = time.monotonic() + 10
            while True:
                if compositor.poll() is not None:
                    raise RuntimeError(f"Sway exited during startup; see {artifacts / 'sway.log'}")
                ipc = list(runtime.glob("sway-ipc.*.sock"))
                displays = [path for path in runtime.glob("wayland-*")
                            if stat.S_ISSOCK(path.stat().st_mode)]
                if len(ipc) == len(displays) == 1:
                    environment.update(SWAYSOCK=str(ipc[0]), WAYLAND_DISPLAY=displays[0].name)
                    try:
                        sway("output HEADLESS-1 mode 1600x1200 scale 1", environment)
                        break
                    except (OSError, subprocess.SubprocessError):
                        pass
                if time.monotonic() >= deadline:
                    raise RuntimeError("Sway did not publish its owned IPC and Wayland sockets")
                time.sleep(0.05)
            # A headless backend has no physical keyboard. Keep a real virtual
            # keyboard on the seat so xdg-shell keyboard focus can be delivered.
            keyboard = subprocess.Popen(
                ["wtype", "a", "-s",
                 str(math.ceil((args.timeout + 15) * 1000))],
                env=environment, stdout=log, stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            time.sleep(0.1)
            if keyboard.poll() is not None:
                raise RuntimeError("the headless seat's virtual keyboard did not start")
            child = subprocess.Popen(command, env=environment, start_new_session=True)
            return child.wait(timeout=args.timeout)
        finally:
            stop_owned_group(child)
            stop_owned_group(keyboard)
            stop_owned_group(compositor)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"with-sway: {error}", file=sys.stderr)
        sys.exit(1)
