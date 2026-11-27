"""Build the static homepage and single-canvas egui demo for GitHub Pages."""

from __future__ import annotations

import argparse
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
DEMO = ROOT / "integration/egui-web-demo"


def resolve_wasm_opt() -> tuple[str, str]:
    executable = shutil.which("wasm-opt")
    if not executable:
        sys.exit(
            "wasm-opt (Binaryen) is required for release builds.\n"
            "Install it with: brew install binaryen (macOS) or from "
            "https://github.com/WebAssembly/binaryen/releases\n"
            "Pass --no-wasm-opt or --dev to skip wasm-opt locally."
        )
    version = subprocess.check_output([executable, "--version"], text=True).strip()
    return executable, version


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dev", action="store_true", help="build without release optimization")
    parser.add_argument(
        "--no-wasm-opt",
        action="store_true",
        help="skip Binaryen wasm-opt post-processing during release builds",
    )
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
    use_wasm_opt = not args.dev and not args.no_wasm_opt
    wasm_opt_exe, wasm_opt_version = resolve_wasm_opt() if use_wasm_opt else (None, "disabled")
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
    fingerprint = hashlib.sha256(wasm.read_bytes())
    for source in [Path(__file__), ROOT / "site/demo/egui/index.html", ROOT / "site/demo/egui/loading.js"]:
        fingerprint.update(source.read_bytes())
    fingerprint.update(installed.encode())
    fingerprint.update(wasm_opt_version.encode())
    build_id = fingerprint.hexdigest()[:16]
    destination = ROOT / "target/site"
    if destination.exists():
        shutil.rmtree(destination)
    shutil.copytree(ROOT / "site", destination)
    (destination / ".nojekyll").touch()
    pkg_dir = destination / "demo/egui/pkg" / build_id
    subprocess.run([
        "wasm-bindgen", str(wasm), "--target", "web", "--no-typescript",
        "--out-name", "egui_web_demo", "--out-dir", str(pkg_dir),
    ], cwd=ROOT, check=True)
    wasm_output = pkg_dir / "egui_web_demo_bg.wasm"
    if wasm_opt_exe is not None:
        optimized = wasm_output.with_suffix(".opt.wasm")
        subprocess.run([
            wasm_opt_exe,
            "-Oz",
            "--strip-debug",
            "--strip-producers",
            "--enable-bulk-memory",
            "--enable-nontrapping-float-to-int",
            "--enable-sign-ext",
            "--enable-mutable-globals",
            "--enable-reference-types",
            "--enable-multivalue",
            str(wasm_output),
            "-o",
            str(optimized),
        ], cwd=ROOT, check=True)
        os.replace(optimized, wasm_output)
    wasm_bytes = wasm_output.stat().st_size
    unhashed_loader = destination / "demo/egui/loading.js"
    shutil.copyfile(ROOT / "site/demo/egui/loading.js", destination / f"demo/egui/loading-{build_id}.js")
    if unhashed_loader.exists():
        unhashed_loader.unlink()
    demo_page = destination / "demo/egui/index.html"
    preloads = (
        f'    <link rel="modulepreload" href="./loading-{build_id}.js">\n'
        f'    <link rel="modulepreload" href="./pkg/{build_id}/egui_web_demo.js">'
    )
    demo_page.write_text(demo_page.read_text(encoding="utf-8").replace(
        'name="dockspace-wasm-bytes" content=""',
        f'name="dockspace-wasm-bytes" content="{wasm_bytes}"',
    ).replace(
        'name="dockspace-build" content=""',
        f'name="dockspace-build" content="{build_id}"',
    ).replace(
        '    <!-- dockspace-preloads -->',
        preloads,
    ), encoding="utf-8")
    print(f"Site built: {destination} (wasm={wasm_bytes:,} bytes, {wasm_bytes / (1024 * 1024):.2f} MiB)")
    print("Preview: python3 -m http.server 8080 --directory target/site")


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        sys.exit(error.returncode)
