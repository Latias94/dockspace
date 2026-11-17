"""Build the static homepage and single-canvas egui demo for GitHub Pages."""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
DEMO = ROOT / "integration/egui-web-demo"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dev", action="store_true", help="build without release optimization")
    args = parser.parse_args()
    try:
        import tomllib
    except ModuleNotFoundError:
        parser.error("Python 3.11 or later is required")
    with (DEMO / "Cargo.lock").open("rb") as source:
        packages = tomllib.load(source)["package"]
    version = next(package["version"] for package in packages if package["name"] == "wasm-bindgen")
    try:
        installed = subprocess.check_output(["wasm-bindgen", "--version"], text=True).strip()
    except FileNotFoundError:
        installed = "not installed"
    if installed != f"wasm-bindgen {version}":
        sys.exit(
            f"wasm-bindgen {version} is required ({installed}).\n"
            f"Install it with: cargo install wasm-bindgen-cli --version {version} --locked"
        )
    target = Path(os.environ.get("CARGO_TARGET_DIR", "target"))
    if not target.is_absolute():
        target = ROOT / target
    command = [
        "cargo", "build", "--manifest-path", str(DEMO / "Cargo.toml"),
        "--target", "wasm32-unknown-unknown", "--target-dir", str(target), "--locked",
    ]
    if not args.dev:
        command.append("--release")
    subprocess.run(command, cwd=ROOT, check=True)
    wasm = target / "wasm32-unknown-unknown" / ("debug" if args.dev else "release") / "dockspace_egui_web_demo.wasm"
    destination = ROOT / "target/site"
    shutil.copytree(ROOT / "site", destination, dirs_exist_ok=True)
    (destination / ".nojekyll").touch()
    subprocess.run([
        "wasm-bindgen", str(wasm), "--target", "web", "--out-name", "egui_web_demo",
        "--out-dir", str(destination / "demo/egui/pkg"),
    ], cwd=ROOT, check=True)
    wasm_bytes = (destination / "demo/egui/pkg/egui_web_demo_bg.wasm").stat().st_size
    demo_page = destination / "demo/egui/index.html"
    demo_page.write_text(demo_page.read_text(encoding="utf-8").replace(
        'name="dockspace-wasm-bytes" content=""',
        f'name="dockspace-wasm-bytes" content="{wasm_bytes}"',
    ), encoding="utf-8")
    print(f"Site built: {destination}")
    print("Preview: python3 -m http.server 8080 --directory target/site")


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        sys.exit(error.returncode)
