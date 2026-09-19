#!/usr/bin/env python3
"""Native release packaging; Python is a maintainer dependency, not a runtime one."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parent.parent
RG = json.loads((ROOT / "scripts/release/ripgrep.json").read_text())
LICENSE_SOURCES = json.loads((ROOT / "scripts/release/license-sources.json").read_text())


def command(args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def download(url, digest):
    cache = ROOT / "target/release-downloads"
    cache.mkdir(parents=True, exist_ok=True)
    destination = cache / digest
    if destination.exists():
        if sha256(destination) != digest:
            raise RuntimeError(f"Cached download checksum mismatch: {url}")
        return destination
    with tempfile.TemporaryDirectory(dir=cache) as work:
        temporary = Path(work) / "download"
        subprocess.run([
            "curl", "--fail", "--location", "--silent", "--show-error",
            "--proto", "=https", "--proto-redir", "=https", "--tlsv1.2",
            "--retry", "3", "--max-time", "120", url, "--output", str(temporary),
        ], check=True)
        if sha256(temporary) != digest:
            raise RuntimeError(f"Download checksum mismatch: {url}")
        temporary.replace(destination)
    return destination


def copy_ripgrep(target, output):
    pin = RG["targets"][target]
    folder = f"ripgrep-{RG['version']}-{pin['archiveTarget']}"
    url = f"https://github.com/BurntSushi/ripgrep/releases/download/{RG['version']}/{folder}.tar.gz"
    archive = download(url, pin["sha256"])
    output.mkdir(parents=True, exist_ok=True)
    # Extract only the expected regular files, never arbitrary archive paths.
    with tarfile.open(archive) as bundle:
        for name in ("rg", "COPYING", "LICENSE-MIT", "UNLICENSE"):
            member = bundle.getmember(f"{folder}/{name}")
            if not member.isfile():
                raise RuntimeError(f"Unexpected ripgrep member: {name}")
            destination = output / (name if name == "rg" else f"licenses/ripgrep/{name}")
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(bundle.extractfile(member).read())
            destination.chmod(0o755 if name == "rg" else 0o644)


def package_metadata(target=None):
    args = ["cargo", "metadata", "--locked", "--format-version", "1"]
    args += ["--filter-platform", target] if target else ["--no-deps"]
    return json.loads(command(args))


def version():
    return next(p["version"] for p in package_metadata()["packages"] if p["name"] == "oko")


def copy_dependency_licenses(target, output):
    metadata = package_metadata(target)
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    selected, pending = set(), [metadata["resolve"]["root"]]
    while pending:
        item = pending.pop()
        if item in selected:
            continue
        selected.add(item)
        pending.extend(dep["pkg"] for dep in nodes[item]["deps"])
    manifest = []
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if package["id"] not in selected or package["name"] == "oko":
            continue
        source = Path(package["manifest_path"]).parent
        name = f"{package['name']}-{package['version']}"
        destination = output / "licenses/dependencies" / name
        destination.mkdir(parents=True)
        found = False
        for path in sorted(source.iterdir()):
            if path.name.lower().startswith(("license", "licence", "copying", "copyright", "notice")):
                if path.is_dir():
                    shutil.copytree(path, destination / path.name)
                else:
                    shutil.copy2(path, destination / path.name)
                found = True
        if package.get("license_file"):
            path = source / package["license_file"]
            if path.is_file():
                shutil.copy2(path, destination / "declared-license.txt")
                found = True
        if not found:
            pin = LICENSE_SOURCES.get(name)
            if not pin:
                raise RuntimeError(f"Missing license text for {name}; pin its upstream license before release")
            shutil.copy2(download(pin["url"], pin["sha256"]), destination / "LICENSE")
        manifest.append({k: package.get(k) for k in ("name", "version", "license", "repository")})
    (output / "licenses/dependencies.json").write_text(json.dumps(manifest, indent=2) + "\n")


def package(target, output, binary):
    host = next(line[6:] for line in command(["rustc", "-vV"]).splitlines() if line.startswith("host: "))
    if host != target:
        raise RuntimeError("Packages must be built and smoke-tested on their native target")
    release_version = version()
    binary = (binary or ROOT / f"target/{target}/release/oko").resolve()
    if command([str(binary), "--version"]) != f"oko {release_version}":
        raise RuntimeError("Binary version differs from Cargo.toml")
    output.mkdir(parents=True, exist_ok=True)
    name = f"oko-v{release_version}-{target}"
    with tempfile.TemporaryDirectory() as work:
        stage = Path(work) / name
        stage.mkdir()
        shutil.copy2(binary, stage / "oko")
        (stage / "oko").chmod(0o755)
        copy_ripgrep(target, stage)
        for file in ("LICENSE", "THIRD_PARTY_NOTICES.md", "README.md"):
            shutil.copy2(ROOT / file, stage / file)
        copy_dependency_licenses(target, stage)
        (stage / "BUILD.json").write_text(json.dumps({
            "version": release_version, "target": target,
            "rustc": command(["rustc", "--version"]),
            "sourceCommit": command(["git", "rev-parse", "HEAD"]),
            "dirty": bool(command(["git", "status", "--porcelain"])),
            "ripgrepVersion": RG["version"],
        }, indent=2) + "\n")
        archive = output / f"{name}.tar.gz"
        with tarfile.open(archive, "w:gz") as bundle:
            for path in [stage, *sorted(stage.rglob("*"))]:
                info = bundle.gettarinfo(str(path), str(path.relative_to(stage.parent)))
                info.uid = info.gid = 0
                info.uname = info.gname = ""
                info.mtime = 0
                if path.is_file():
                    with path.open("rb") as data:
                        bundle.addfile(info, data)
                else:
                    bundle.addfile(info)
        archive.with_name(archive.name + ".sha256").write_text(f"{sha256(archive)}  {archive.name}\n")
    print(archive)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    tag = commands.add_parser("check-tag")
    tag.add_argument("tag")
    for name in ("ripgrep", "package"):
        sub = commands.add_parser(name)
        sub.add_argument("--target", choices=RG["targets"], required=True)
        sub.add_argument("--output", type=Path, required=True)
        if name == "package":
            sub.add_argument("--binary", type=Path)
    args = parser.parse_args()
    if args.command == "check-tag":
        if args.tag != f"v{version()}":
            parser.error("Release tag must equal v followed by Cargo.toml's package version")
    elif args.command == "ripgrep":
        copy_ripgrep(args.target, args.output.resolve())
    else:
        package(args.target, args.output.resolve(), args.binary)


if __name__ == "__main__":
    main()
