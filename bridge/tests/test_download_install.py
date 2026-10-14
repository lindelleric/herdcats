"""Run the public shell installer with a local release mirror and real helper."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / 'scripts/install.sh'
BINARY = ROOT / 'target/release/herdcats-bridge'

class DownloadInstallTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='hc-download-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.version = json.loads(subprocess.check_output([str(BINARY), '--version-json']))['version']
        self.bin = self.root / 'home with spaces/.local/bin'
        self.mirror = self.root / 'release'
        self.mirror.mkdir()
        self.asset = self.mirror / 'herdcats-bridge-darwin-arm64'
        shutil.copyfile(BINARY, self.asset)
        self.checksums()
        commands = self.root / 'commands'
        commands.mkdir()
        (commands / 'uname').write_text('#!/bin/sh\ncase "$1" in -s) echo Darwin;; -m) echo arm64;; esac\n')
        (commands / 'curl').write_text('''#!/usr/bin/env python3
import os, pathlib, shutil, sys
args = sys.argv[1:]
out = args[args.index('-o') + 1]
url = args[-1]
shutil.copyfile(pathlib.Path(os.environ['MIRROR']) / url.rsplit('/', 1)[1], out)
if os.environ.get('RACE') and 'herdcats-bridge-darwin' in url:
    dest = pathlib.Path(os.environ['HOME']) / '.local/bin/herdcats-bridge'
    dest.parent.mkdir(parents=True, exist_ok=True)
    if os.environ['RACE'] == 'directory': dest.mkdir()
    else: dest.symlink_to(os.environ['OUTSIDE'], target_is_directory=True)
''')
        for command in commands.iterdir():
            command.chmod(0o755)
        self.env = {**os.environ, 'PATH': str(commands) + ':' + os.environ['PATH'],
                    'MIRROR': str(self.mirror), 'HOME': str(self.root / 'home with spaces')}

    def checksums(self):
        digest = hashlib.sha256(self.asset.read_bytes()).hexdigest()
        (self.mirror / 'SHA256SUMS').write_text(digest + '  ' + self.asset.name + '\n')

    def run_install(self, *extra):
        return subprocess.run(['/bin/sh', str(SCRIPT), '--version', self.version, *extra],
                              env=self.env, text=True, capture_output=True)

    def test_download_verify_install_without_compiler_or_python_dependency(self):
        result = self.run_install()
        self.assertEqual(result.returncode, 0, result.stderr)
        installed = self.bin / 'herdcats-bridge'
        self.assertEqual(installed.read_bytes(), self.asset.read_bytes())
        self.assertEqual(list(self.bin.iterdir()), [installed])
        self.assertIn(self.version, subprocess.check_output([str(installed), '--version-json'], text=True))

    def test_bad_checksum_never_executes_or_installs_download(self):
        marker = self.root / 'executed'
        self.asset.write_text('#!/bin/sh\ntouch ' + str(marker) + '\n')
        result = self.run_install()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(marker.exists())
        self.assertFalse(self.bin.exists())

    def test_update_is_explicit_and_symlinks_are_preserved(self):
        self.assertEqual(self.run_install().returncode, 0)
        self.assertNotEqual(self.run_install().returncode, 0)
        self.assertEqual(self.run_install('--replace').returncode, 0)
        installed = self.bin / 'herdcats-bridge'
        installed.unlink()
        target = self.root / 'keep'
        target.write_text('keep')
        installed.symlink_to(target)
        self.assertNotEqual(self.run_install('--replace').returncode, 0)
        self.assertEqual(target.read_text(), 'keep')

    def test_destination_directory_races_do_not_redirect_install(self):
        for kind in ['directory', 'symlink']:
            with self.subTest(kind=kind):
                self.env['HOME'] = str(self.root / ('race-home-' + kind))
                self.bin = Path(self.env['HOME']) / '.local/bin'
                outside = self.root / ('outside-' + kind)
                outside.mkdir()
                self.env['RACE'] = kind
                self.env['OUTSIDE'] = str(outside)
                result = self.run_install()
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertEqual(list(outside.iterdir()), [])
                dest = self.bin / 'herdcats-bridge'
                if kind == 'directory':
                    self.assertEqual(list(dest.iterdir()), [])
                    dest.rmdir()
                else:
                    self.assertTrue(dest.is_symlink())
                    dest.unlink()

    def test_replace_directory_race_cannot_move_into_directory(self):
        self.assertEqual(self.run_install().returncode, 0)
        commands = Path(self.env['PATH'].split(':')[0])
        real_mv = shutil.which('mv')
        wrapper = commands / 'mv'
        wrapper.write_text('#!/usr/bin/env python3\nimport os, pathlib, sys\ndest = pathlib.Path(os.environ["HOME"]) / ".local/bin/herdcats-bridge"\ndest.unlink()\ndest.mkdir()\nos.execv(' + repr(real_mv) + ', ["mv"] + sys.argv[1:])\n')
        wrapper.chmod(0o755)
        self.assertNotEqual(self.run_install('--replace').returncode, 0)
        self.assertEqual(list((self.bin / 'herdcats-bridge').iterdir()), [])

    def test_missing_asset_leaves_host_untouched(self):
        self.asset.unlink()
        self.assertNotEqual(self.run_install().returncode, 0)
        self.assertFalse(self.bin.exists())

    def test_platform_and_version_are_validated_before_download(self):
        commands = Path(self.env['PATH'].split(':')[0])
        (commands / 'uname').write_text('#!/bin/sh\necho unsupported\n')
        self.assertNotEqual(self.run_install().returncode, 0)
        self.assertFalse(self.bin.exists())
        self.assertNotEqual(self.run_install('--version', '../bad').returncode, 0)

if __name__ == '__main__':
    unittest.main()
