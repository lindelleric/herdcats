import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]

class PackageTests(unittest.TestCase):
    def test_package_contains_exact_binary_and_checksum(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / 'release'
            binary = ROOT / 'target/release/herdcats-bridge'
            version = json.loads(subprocess.check_output([str(binary), '--version-json']))['version']
            result = subprocess.run([sys.executable, str(ROOT / 'scripts/package.py'),
                '--binary', str(binary), '--platform', 'darwin-arm64', '--version', version, '--out', str(out)], capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            asset = out / 'herdcats-bridge-darwin-arm64'
            self.assertEqual(asset.read_bytes(), binary.read_bytes())
            self.assertEqual((out / 'SHA256SUMS-darwin-arm64').read_text(), hashlib.sha256(binary.read_bytes()).hexdigest() + '  herdcats-bridge-darwin-arm64\n')
            result = subprocess.run([sys.executable, str(ROOT / 'scripts/package.py'),
                '--binary', str(binary), '--platform', 'darwin-arm64', '--version', '9.9.9', '--out', str(Path(tmp) / 'wrong')], capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((Path(tmp) / 'wrong').exists())

    def test_released_installer_defaults_to_packaged_version(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            binary = root / 'future'
            binary.write_text("#!/bin/sh\necho '{\"version\":\"0.2.0\",\"protocol\":1}'\n")
            binary.chmod(0o755)
            out = root / 'release'
            result = subprocess.run([sys.executable, str(ROOT / 'scripts/package.py'), '--binary', str(binary), '--platform', 'darwin-arm64', '--version', '0.2.0', '--out', str(out)], capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            commands = root / 'commands'
            commands.mkdir()
            curl = commands / 'curl'
            curl.write_text('#!/usr/bin/env python3\nimport os, sys\nopen(os.environ["URL_LOG"], "w").write(sys.argv[-1])\nsys.exit(22)\n')
            curl.chmod(0o755)
            log = root / 'url'
            subprocess.run(['/bin/sh', str(out / 'install.sh')], env={**os.environ, 'HOME': str(root), 'PATH': str(commands) + ':' + os.environ['PATH'], 'URL_LOG': str(log)}, capture_output=True)
            self.assertTrue(log.exists(), 'Packaged installer must execute')
            self.assertIn('herdcats-bridge-v0.2.0/SHA256SUMS', log.read_text())

if __name__ == '__main__':
    unittest.main()
