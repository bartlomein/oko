#!/usr/bin/env python3
"""Offline installer integration tests. No user settings, network, or credentials."""
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest

INSTALLER = Path(__file__).resolve().parents[1] / "install.sh"
REAL_ARCHIVE = Path(os.environ["OKO_TEST_ARCHIVE"]).resolve() if os.environ.get("OKO_TEST_ARCHIVE") else None


class InstallerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="oko installer '")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.mock = self.root / "tools"
        self.mock.mkdir()
        self.downloads = self.root / "downloads"
        self.downloads.mkdir()
        self.home = self.root / "home"
        self.home.mkdir()
        self.bin = self.home / ".local/bin"
        self.install = self.home / ".local/share/oko"
        self.env = {k: v for k, v in os.environ.items() if not k.startswith("OKO_")}
        self.env.update(HOME=str(self.home), PATH=str(self.mock) + os.pathsep + os.environ["PATH"],
                        FIXTURES=str(self.downloads), TEST_OS="Darwin", TEST_ARCH="arm64",
                        OKO_INSTALL_DIR=str(self.install), OKO_BIN_DIR=str(self.bin), OKO_VERSION="v0.2.0")
        self.stub("uname", '#!/bin/sh\ncase "$1" in -s) echo "$TEST_OS";; -m) echo "$TEST_ARCH";; esac\n')
        self.stub("getconf", '#!/bin/sh\necho "${TEST_LIBC:-glibc 2.35}"\n')
        # Only intercept the transport; exercise real tar, checksum, filesystem, shell.
        self.stub("curl", f'''#!{sys.executable}
import os,sys,shutil
from pathlib import Path
args=sys.argv[1:]
assert '--proto' in args and args[args.index('--proto')+1]=='=https'
url=next(a for a in args if a.startswith('https://'))
assert url.startswith('https://github.com/bartlomein/oko/releases/download/')
name=url.rsplit('/',1)[1]
p=Path(os.environ['FIXTURES'])/name
if os.environ.get('FAIL_DOWNLOAD') or not p.exists(): sys.exit(22)
shutil.copyfile(p,args[args.index('--output')+1])
''')

    def stub(self, name, text):
        p = self.mock / name
        p.write_text(text)
        p.chmod(0o755)

    def fixture(self, target="aarch64-apple-darwin", version="v0.2.0", reported=None,
                malicious=None, missing_rg=False):
        bundle = f"oko-{version}-{target}"
        archive = self.downloads / (bundle + ".tar.gz")
        with tarfile.open(archive, "w:gz") as tar:
            directory = tarfile.TarInfo(bundle + "/")
            directory.type = tarfile.DIRTYPE
            directory.mode = 0o755
            tar.addfile(directory)
            for name, value in [("oko", reported or f"oko {version[1:]}"), ("rg", "ripgrep 15.2.0")]:
                if missing_rg and name == "rg":
                    continue
                data = f"#!/bin/sh\necho '{value}'\n".encode()
                info = tarfile.TarInfo(bundle + "/" + name)
                info.mode = 0o755
                info.size = len(data)
                tar.addfile(info, io.BytesIO(data))
            if malicious:
                info = tarfile.TarInfo(malicious)
                if malicious.endswith("link"):
                    info.type = tarfile.SYMTYPE
                    info.linkname = "/tmp"
                tar.addfile(info)
        self.checksum(archive)
        return archive

    def checksum(self, archive):
        (self.downloads / "SHA256SUMS").write_text(
            hashlib.sha256(archive.read_bytes()).hexdigest() + "  " + archive.name + "\n")

    def run_install(self, succeeds=True):
        # Exercise the curl | sh entry mode rather than just running a script file.
        result = subprocess.run(["sh"], input=INSTALLER.read_text(), env=self.env,
                                cwd=self.home, text=True, capture_output=True, timeout=20)
        self.assertEqual(result.returncode == 0, succeeds, result.stdout + result.stderr)
        return result

    def test_platforms_and_repeat_install(self):
        for osname, arch, target in [
            ("Darwin", "arm64", "aarch64-apple-darwin"),
            ("Darwin", "x86_64", "x86_64-apple-darwin"),
            ("Linux", "aarch64", "aarch64-unknown-linux-gnu"),
            ("Linux", "x86_64", "x86_64-unknown-linux-gnu"),
        ]:
            with self.subTest(target=target):
                self.env.update(TEST_OS=osname, TEST_ARCH=arch)
                self.fixture(target)
                result = self.run_install()
                installed = self.bin / "oko"
                self.assertTrue(installed.is_symlink())
                self.assertTrue((installed.resolve().parent / "rg").is_file())
                self.assertFalse((self.bin / "rg").exists())
                # Installer prints a correctly quoted PATH command, even for apostrophes/spaces.
                export = next(line for line in result.stdout.splitlines() if line.startswith("export PATH="))
                check = subprocess.run(["sh", "-c", export + '; command -v oko'], env=self.env,
                                       text=True, capture_output=True, check=True)
                self.assertEqual(check.stdout.strip(), str(installed))

    def test_existing_unmanaged_install_untouched(self):
        self.bin.mkdir(parents=True)
        (self.bin / "oko").write_text("original")
        self.run_install(False)
        self.assertEqual((self.bin / "oko").read_text(), "original")
        (self.bin / "oko").unlink()
        (self.bin / "oko").symlink_to(self.root / "other")
        self.run_install(False)
        self.assertEqual(os.readlink(self.bin / "oko"), str(self.root / "other"))

    def test_failures_preserve_working_install(self):
        archive = self.fixture()
        self.run_install()
        old = (self.bin / "oko").resolve()
        for failure in ["download", "checksum", "missing_checksum", "duplicate_checksum",
                        "version", "missing_rg", "traversal", "link"]:
            with self.subTest(failure=failure):
                self.env.pop("FAIL_DOWNLOAD", None)
                archive = self.fixture(reported="oko 9.9.9" if failure == "version" else None,
                                       missing_rg=failure == "missing_rg",
                                       malicious="../escaped" if failure == "traversal" else
                                       "oko-v0.2.0-aarch64-apple-darwin/link" if failure == "link" else None)
                if failure == "download": self.env["FAIL_DOWNLOAD"] = "1"
                if failure == "checksum": archive.write_bytes(b"corrupt")
                sums = self.downloads / "SHA256SUMS"
                if failure == "missing_checksum": sums.write_text("")
                if failure == "duplicate_checksum": sums.write_text(sums.read_text() * 2)
                self.run_install(False)
                self.assertEqual((self.bin / "oko").resolve(), old)
                self.assertEqual(list((self.install / "releases").iterdir()), [old.parent.parent])

    def test_unsupported_platforms_and_invalid_options(self):
        for overrides in [{"TEST_OS": "Windows"}, {"TEST_ARCH": "riscv64"},
                          {"TEST_OS": "Linux", "TEST_LIBC": "musl"},
                          {"TEST_OS": "Linux", "TEST_LIBC": "glibc 2.34"},
                          {"OKO_VERSION": "../../evil"}, {"OKO_BIN_DIR": "relative"}]:
            with self.subTest(overrides=overrides):
                previous = self.env.copy()
                self.env.update(overrides)
                self.run_install(False)
                self.env = previous
                self.assertFalse((self.bin / "oko").exists())

    def test_shasum_fallback_and_existing_ripgrep(self):
        if not shutil.which("shasum"):
            self.skipTest("shasum not installed")
        # GNU tar invokes gzip through PATH when reading .tar.gz archives.
        for name in ["tar", "gzip", "awk", "grep", "mktemp", "readlink", "mkdir", "rm", "ln", "mv", "sed", "shasum"]:
            (self.mock / name).symlink_to(shutil.which(name))
        self.env["PATH"] = str(self.mock)
        self.bin.mkdir(parents=True)
        (self.bin / "rg").write_text("user ripgrep")
        self.fixture()
        # Use an absolute shell because this test intentionally restricts PATH.
        result = subprocess.run(["/bin/sh"], input=INSTALLER.read_text(), env=self.env,
                                text=True, capture_output=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((self.bin / "rg").read_text(), "user ripgrep")

    def test_explicit_version(self):
        self.env["OKO_VERSION"] = "0.3.0-rc.1"
        self.fixture(version="v0.3.0-rc.1")
        self.run_install()
        self.assertIn("v0.3.0-rc.1", str((self.bin / "oko").resolve()))

    @unittest.skipUnless(REAL_ARCHIVE, "Set OKO_TEST_ARCHIVE to test a native release")
    def test_native_archive(self):
        self.env["TEST_OS"] = os.uname().sysname
        self.env["TEST_ARCH"] = os.uname().machine
        archive = self.downloads / REAL_ARCHIVE.name
        shutil.copyfile(REAL_ARCHIVE, archive)
        with tarfile.open(archive) as tar:
            metadata = next(m for m in tar.getmembers() if m.name.endswith("/BUILD.json"))
            self.env["OKO_VERSION"] = json.load(tar.extractfile(metadata))["version"]
        self.checksum(archive)
        self.run_install()
        project = self.root / "project"
        project.mkdir()
        (project / "example.rs").write_text("pub fn verify_archive_checksum() {}\n")
        result = subprocess.run([str(self.bin / "oko"), "ask", "archive checksum", "--no-jev"],
                                cwd=project, env={**self.env, "OKO_NO_CACHE": "1"},
                                capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("example.rs", result.stdout)


if __name__ == "__main__":
    unittest.main()
