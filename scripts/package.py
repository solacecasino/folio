#!/usr/bin/env python3
"""Builds text9000 for this computer and packs the download into dist/.

    macOS    dist/text9000-<version>-macos-arm64.zip      text9000.app
             (UNIVERSAL=1: text9000-<version>-macos-universal.zip, for Intel Macs too)
    Windows  dist/text9000-<version>-windows-x64.zip      text9000.exe, themes, README
    Linux    dist/text9000-<version>-linux-x64.tar.gz     text9000, themes, install.sh

--lite builds text9000 Lite instead: the same app without the WebAssembly
plugin runtime (cargo's --no-default-features), named text9000-lite-... .

Usage: python3 scripts/package.py [--lite]
"""

import os
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DIST = ROOT / "dist"


def run(*args):
    print("+", " ".join(str(a) for a in args), flush=True)
    subprocess.run([str(a) for a in args], cwd=ROOT, check=True)


def arch():
    machine = platform.machine().lower()
    return {"amd64": "x64", "x86_64": "x64", "aarch64": "arm64", "arm64": "arm64"}.get(machine, machine)


def zip_dir(folder: Path, out: Path):
    """Zips a folder as one top-level folder, keeping executable bits."""
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
        for path in sorted(folder.rglob("*")):
            name = path.relative_to(folder.parent).as_posix()
            if path.is_dir():
                continue
            info = zipfile.ZipInfo.from_file(path, name)
            info.compress_type = zipfile.ZIP_DEFLATED
            with open(path, "rb") as f:
                z.writestr(info, f.read())


def main():
    version = re.search(r'^version = "(.+)"', (ROOT / "Cargo.toml").read_text(), re.M).group(1)
    system = platform.system()
    lite = "--lite" in sys.argv
    flags = ["--no-default-features"] if lite else []
    edition = "lite-" if lite else ""
    # Both editions can be packed one after the other into the same dist/.
    DIST.mkdir(exist_ok=True)

    if system == "Darwin":
        env = dict(os.environ, CARGO_FLAGS=" ".join(flags))
        print("+ sh scripts/bundle.sh", " ".join(flags), flush=True)
        subprocess.run(["sh", "scripts/bundle.sh"], cwd=ROOT, check=True, env=env)
        mac = "universal" if os.environ.get("UNIVERSAL") == "1" else arch()
        out = DIST / f"text9000-{edition}{version}-macos-{mac}.zip"
        # ditto keeps the app bundle and its signature intact.
        run("ditto", "-c", "-k", "--norsrc", "--noextattr", "--keepParent", "target/text9000.app", out)

    elif system == "Windows":
        run("cargo", "build", "--release", *flags)
        stage = DIST / f"text9000-{edition}{version}-windows-{arch()}"
        shutil.rmtree(stage, ignore_errors=True)
        stage.mkdir()
        shutil.copy2(ROOT / "target/release/text9000.exe", stage)
        shutil.copytree(ROOT / "themes", stage / "themes")
        shutil.copy2(ROOT / "packaging/windows/README.txt", stage)
        shutil.copy2(ROOT / "LICENSE", stage / "LICENSE.txt")
        out = DIST / f"{stage.name}.zip"
        zip_dir(stage, out)
        shutil.rmtree(stage)

    elif system == "Linux":
        run("cargo", "build", "--release", *flags)
        stage = DIST / f"text9000-{edition}{version}-linux-{arch()}"
        shutil.rmtree(stage, ignore_errors=True)
        stage.mkdir()
        shutil.copy2(ROOT / "target/release/text9000", stage)
        shutil.copytree(ROOT / "themes", stage / "themes")
        for f in ["install.sh", "text9000.desktop", "text9000-mime.xml"]:
            shutil.copy2(ROOT / "packaging/linux" / f, stage)
        shutil.copy2(ROOT / "assets/icon-256.png", stage / "text9000.png")
        shutil.copy2(ROOT / "LICENSE", stage)
        os.chmod(stage / "text9000", 0o755)
        os.chmod(stage / "install.sh", 0o755)
        out = DIST / f"{stage.name}.tar.gz"
        with tarfile.open(out, "w:gz") as t:
            t.add(stage, arcname=stage.name)
        shutil.rmtree(stage)

    else:
        sys.exit(f"Don't know how to package for {system}")

    print(f"Built {out.relative_to(ROOT)} ({out.stat().st_size // 1024} KB)")


if __name__ == "__main__":
    main()
