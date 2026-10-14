import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import tempfile
import textwrap
import unittest

ROOT = Path(__file__).resolve().parents[1]


class ReleaseSourceTests(unittest.TestCase):
    def test_workflow_rejects_source_that_does_not_match_version_tag(self):
        workflow = (ROOT.parent / '.github/workflows/bridge-release.yml').read_text()
        for step in ['Verify version and native architecture', 'Resolve release version']:
            # Exercise the workflow's actual validation against a local Git repo.
            block = workflow.split('name: ' + step + '\n', 1)[1]
            script = textwrap.dedent(block.split("python3 - <<'PY'\n", 1)[1].split('\n          PY', 1)[0])
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                (root / 'bridge').mkdir()
                (root / 'bridge/Cargo.toml').write_text('[package]\nversion = "0.2.0"\n')
                def git(*args):
                    return subprocess.check_output(['git', *args], cwd=root, text=True).strip()
                git('init', '-q')
                git('add', '.')
                git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.com', 'commit', '-qm', 'release')
                tagged = git('rev-parse', 'HEAD')
                git('tag', 'herdcats-bridge-v0.2.0')
                env = {**os.environ, 'BRIDGE_VERSION': '0.2.0',
                    'BRIDGE_PLATFORM': {'Darwin': 'darwin', 'Linux': 'linux'}[platform.system()] + '-' + {'aarch64': 'arm64', 'arm64': 'arm64', 'x86_64': 'x86_64'}[platform.machine()],
                    'GITHUB_EVENT_NAME': 'workflow_dispatch', 'GITHUB_SHA': tagged,
                    'GITHUB_REF': 'refs/heads/main', 'GITHUB_REF_NAME': 'main',
                    'GITHUB_ENV': str(root / 'env')}
                def validate():
                    return subprocess.run([sys.executable, '-c', script], cwd=root, env=env, capture_output=True)
                with self.subTest(step=step, source='matching'):
                    result = validate()
                    self.assertEqual(result.returncode, 0, result.stderr)
                with self.subTest(step=step, source='matching-tag-push'):
                    env.update(GITHUB_EVENT_NAME='push', BRIDGE_VERSION='',
                               GITHUB_REF='refs/tags/herdcats-bridge-v0.2.0',
                               GITHUB_REF_NAME='herdcats-bridge-v0.2.0')
                    result = validate()
                    self.assertEqual(result.returncode, 0, result.stderr)
                with self.subTest(step=step, source='wrong-tag-push'):
                    env['GITHUB_REF'] = 'refs/tags/unrelated'
                    self.assertNotEqual(validate().returncode, 0)
                env.update(GITHUB_EVENT_NAME='workflow_dispatch', BRIDGE_VERSION='0.2.0',
                           GITHUB_REF='refs/heads/main', GITHUB_REF_NAME='main')
                with self.subTest(step=step, source='missing-version-tag'):
                    env['BRIDGE_VERSION'] = '0.3.0'
                    self.assertNotEqual(validate().returncode, 0)
                env['BRIDGE_VERSION'] = '0.2.0'
                with self.subTest(step=step, source='mismatched-dispatch'):
                    env['GITHUB_SHA'] = '0' * 40
                    self.assertNotEqual(validate().returncode, 0)
                env['GITHUB_SHA'] = tagged
                (root / 'unrelated').write_text('branch work')
                git('add', '.')
                git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.com', 'commit', '-qm', 'unrelated')
                env['GITHUB_SHA'] = git('rev-parse', 'HEAD')
                with self.subTest(step=step, source='same-version-wrong-head'):
                    self.assertNotEqual(validate().returncode, 0)

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
