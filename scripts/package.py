#!/usr/bin/env python3
"""Builds Folio for this computer and packs the download into dist/.

    macOS    dist/Folio-<version>-macos-arm64.zip     Folio.app
    Windows  dist/Folio-<version>-windows-x64.zip     folio.exe, themes, README
    Linux    dist/folio-<version>-linux-x64.tar.gz    folio, themes, install.sh

Usage: python3 scripts/package.py
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
    shutil.rmtree(DIST, ignore_errors=True)
    DIST.mkdir()

    if system == "Darwin":
        run("sh", "scripts/bundle.sh")
        out = DIST / f"Folio-{version}-macos-{arch()}.zip"
        # ditto keeps the app bundle and its signature intact.
        run("ditto", "-c", "-k", "--norsrc", "--noextattr", "--keepParent", "target/Folio.app", out)

    elif system == "Windows":
        run("cargo", "build", "--release")
        stage = DIST / f"Folio-{version}-windows-{arch()}"
        stage.mkdir()
        shutil.copy2(ROOT / "target/release/folio.exe", stage)
        shutil.copytree(ROOT / "themes", stage / "themes")
        shutil.copy2(ROOT / "packaging/windows/README.txt", stage)
        shutil.copy2(ROOT / "LICENSE", stage / "LICENSE.txt")
        out = DIST / f"{stage.name}.zip"
        zip_dir(stage, out)
        shutil.rmtree(stage)

    elif system == "Linux":
        run("cargo", "build", "--release")
        stage = DIST / f"folio-{version}-linux-{arch()}"
        stage.mkdir()
        shutil.copy2(ROOT / "target/release/folio", stage)
        shutil.copytree(ROOT / "themes", stage / "themes")
        for f in ["install.sh", "folio.desktop"]:
            shutil.copy2(ROOT / "packaging/linux" / f, stage)
        shutil.copy2(ROOT / "assets/icon-256.png", stage / "folio.png")
        shutil.copy2(ROOT / "LICENSE", stage)
        os.chmod(stage / "folio", 0o755)
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
