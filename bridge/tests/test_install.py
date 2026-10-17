"""Exercise explicit installation against temporary homes and real binaries."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(os.environ.get('HERDCATS_TEST_BINARY', ROOT / 'target/debug/herdcats-bridge'))
INSTALLER = ROOT / 'scripts/install.py'

class InstallTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='herdcats-install-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.destination = self.root / 'bin with spaces'

    def install(self, binary=BINARY, *extra):
        return subprocess.run([sys.executable, str(INSTALLER), '--binary', str(binary),
                               '--bin-dir', str(self.destination), *extra],
                              capture_output=True, text=True, stdin=subprocess.DEVNULL)

    def test_installed_binary_is_executable_and_reports_version(self):
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        installed = self.destination / 'herdcats-bridge'
        self.assertEqual(installed.read_bytes(), BINARY.read_bytes())
        version = subprocess.check_output([str(installed), '--version-json'], text=True)
        self.assertEqual(json.loads(version)['protocol'], 1)
        self.assertEqual(list(self.destination.iterdir()), [installed])

    def test_existing_file_requires_explicit_replace(self):
        self.destination.mkdir()
        installed = self.destination / 'herdcats-bridge'
        installed.write_text('existing installation')
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(installed.read_text(), 'existing installation')
        result = self.install(BINARY, '--replace')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(installed.read_bytes(), BINARY.read_bytes())

    def test_symlink_is_rejected_even_with_replace(self):
        self.destination.mkdir()
        target = self.root / 'important'
        target.write_text('keep')
        (self.destination / 'herdcats-bridge').symlink_to(target)
        self.assertNotEqual(self.install(BINARY, '--replace').returncode, 0)
        self.assertEqual(target.read_text(), 'keep')
        self.assertTrue((self.destination / 'herdcats-bridge').is_symlink())

    def test_incompatible_binary_does_not_create_destination(self):
        binary = self.root / 'wrong-bridge'
        binary.write_text('#!/bin/sh\nprintf \'{"name":"herdcats-bridge","version":"99.0","protocol":99,"herdr_protocol":22,"agents":["claude"]}\\n\'\n')
        binary.chmod(0o755)
        self.assertNotEqual(self.install(binary).returncode, 0)
        self.assertFalse(self.destination.exists())

    def test_invalid_binary_does_not_create_destination(self):
        binary = self.root / 'invalid'
        binary.write_text('not an executable')
        self.assertNotEqual(self.install(binary).returncode, 0)
        self.assertFalse(self.destination.exists())

    def test_replace_rejects_fifo_destination(self):
        self.destination.mkdir()
        installed = self.destination / 'herdcats-bridge'
        os.mkfifo(installed)
        self.assertNotEqual(self.install(BINARY, '--replace').returncode, 0)
        import stat
        self.assertTrue(stat.S_ISFIFO(installed.lstat().st_mode))

    def test_probe_output_is_bounded_on_both_streams(self):
        for stream in ['stdout', 'stderr']:
            with self.subTest(stream=stream):
                binary = self.root / ('large-' + stream)
                binary.write_text('#!/usr/bin/env python3\nimport sys, time\nwhile True:\n sys.' + stream + '.write("x" * 10000)\n sys.' + stream + '.flush()\n time.sleep(.01)\n')
                binary.chmod(0o755)
                start = time.monotonic()
                self.assertNotEqual(self.install(binary).returncode, 0)
                self.assertLess(time.monotonic() - start, 3, "Excess output must fail before the five-second timeout")
                self.assertFalse(self.destination.exists())

    def test_probe_timeout_reaps_child(self):
        pid_file = self.root / 'probe.pid'
        binary = self.root / 'sleepy'
        binary.write_text('#!/usr/bin/env python3\nimport os, time\nopen(' + repr(str(pid_file)) + ', "w").write(str(os.getpid()))\ntime.sleep(30)\n')
        binary.chmod(0o755)
        self.assertNotEqual(self.install(binary).returncode, 0)
        pid = int(pid_file.read_text())
        with self.assertRaises(ProcessLookupError):
            os.kill(pid, 0)
        self.assertFalse(self.destination.exists())

    def test_no_implicit_install_destination(self):
        result = subprocess.run([sys.executable, str(INSTALLER), '--binary', str(BINARY)],
                                env={**os.environ, 'HOME': str(self.root)}, capture_output=True,
                                stdin=subprocess.DEVNULL)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.root / '.local').exists())


if __name__ == '__main__':
    unittest.main()
