"""Run the actual app installer wrapper on an isolated host with a fake HTTPS transport."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import textwrap
import unittest

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / 'ios/Herdcats/Networking/AgentBridgeClient.swift'

class MobileHelperInstallationTests(unittest.TestCase):
    def script(self, replace=False):
        source = SOURCE.read_text()
        body = re.search(r'static func installationScript.*?return #"""(.*?)"""#', source, re.S)[1]
        version = re.search(r'static let version = "([^"]+)"', source)[1]
        digest = re.search(r'static let installerSHA256 = "([^"]+)"', source)[1]
        return textwrap.dedent(body).replace(r'\#(version)', version).replace(r'\#(installerSHA256)', digest).replace(r'\#(replace ? " --replace" : "")', ' --replace' if replace else '')

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.home = self.root / 'home'
        self.home.mkdir()
        self.destination = self.home / '.local/bin/herdcats-bridge'
        self.fake = self.root / 'bin'
        self.fake.mkdir()
        source = SOURCE.read_text()
        version = re.search(r'static let version = "([^"]+)"', source)[1]
        agents = ['claude', 'codex'] if version != '0.1.4' else ['claude']
        self.report = {'agents': agents, 'herdr_protocol': 22, 'name': 'herdcats-bridge', 'protocol': 1, 'version': version}
        self.binary = self.root / 'binary'
        self.binary.write_text('#!/bin/sh\nprintf \'%s\\n\' \'' + json.dumps(self.report, separators=(',', ':')) + "'\n")
        self.binary.chmod(0o755)
        curl = self.fake / 'curl'
        curl.write_text('''#!/usr/bin/env python3
import hashlib, os, pathlib, sys
args=sys.argv[1:]
url=next(x for x in args if x.startswith('https://'))
if os.environ.get('HC_CASE') == 'download-failed': sys.exit(22)
out=pathlib.Path(args[args.index('-o')+1])
if url.endswith('/install.sh'):
 data=pathlib.Path(os.environ['HC_INSTALLER']).read_bytes()
 if os.environ.get('HC_CASE') == 'tampered': data=b'#!/bin/sh\\ntouch "$HOME/UNTRUSTED_EXECUTED"\\n'
elif url.endswith('/SHA256SUMS'):
 binary=pathlib.Path(os.environ['HC_BINARY']).read_bytes()
 digest=hashlib.sha256(binary).hexdigest()
 data=''.join(digest+'  herdcats-bridge-'+p+'\\n' for p in ['darwin-arm64','darwin-x86_64','linux-arm64','linux-x86_64']).encode()
else: data=pathlib.Path(os.environ['HC_BINARY']).read_bytes()
out.write_bytes(data)
''')
        curl.chmod(0o755)
        installer_path = ROOT / 'bridge/scripts/install.sh'
        digest = re.search(r'static let installerSHA256 = "([^"]+)"', source)[1]
        if hashlib.sha256(installer_path.read_bytes()).hexdigest() != digest:
            text = re.sub(r'^HB_VERSION=.*$', f'HB_VERSION={version}', installer_path.read_text(), flags=re.M)
            if version.startswith('0.1.'):
                text = re.sub(
                    r'case "\$HB_VERSION" in\n  0\.1\.\*\) HB_CONTRACT=.*?\n  \*\) HB_CONTRACT=.*?\nesac',
                    'HB_CONTRACT=\'{"agents":["claude"],"herdr_protocol":22,"name":"herdcats-bridge","protocol":1,"version":"\'"$HB_VERSION"\'"}\'',
                    text,
                )
            pinned = self.root / 'pinned_install.sh'
            pinned.write_text(text)
            rebuilt_digest = hashlib.sha256(pinned.read_bytes()).hexdigest()
            self.assertEqual(
                rebuilt_digest,
                digest,
                f'Rebuilt installer for pinned version {version} sha256 {rebuilt_digest} does not match pinned installerSHA256 {digest}',
            )
            installer_path = pinned
        self.env = {**os.environ, 'HOME':str(self.home), 'PATH':str(self.fake)+':'+os.environ['PATH'], 'HC_INSTALLER':str(installer_path), 'HC_BINARY':str(self.binary), 'TMPDIR':str(self.root)}

    def run_install(self, replace=False, case=''):
        return subprocess.run(['/bin/sh','-c',self.script(replace)], env={**self.env,'HC_CASE':case}, capture_output=True,text=True,timeout=10)

    def test_explicit_install_creates_verified_helper_and_reports_version(self):
        result = self.run_install()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.destination.is_file(), result.stdout)
        self.assertEqual(self.destination.read_bytes(), self.binary.read_bytes())
        self.assertIn('HC_HELPER_INSTALLED', result.stdout)
        ver = self.report['version']
        self.assertIn(f'"version":"{ver}"', result.stdout)

    def test_untrusted_download_is_never_executed(self):
        result = self.run_install(case='tampered')
        self.assertIn('HC_HELPER_ERROR:checksum_mismatch', result.stdout)
        self.assertFalse(self.destination.exists())
        self.assertFalse((self.home/'UNTRUSTED_EXECUTED').exists())

    def test_download_failure_does_not_install(self):
        result = self.run_install(case='download-failed')
        self.assertIn('HC_HELPER_ERROR:download_failed', result.stdout)
        self.assertFalse(self.destination.exists())

    def test_existing_regular_helper_requires_explicit_replacement(self):
        self.destination.parent.mkdir(parents=True)
        self.destination.write_text('existing user installation')
        result = self.run_install()
        self.assertIn('HC_HELPER_ERROR:install_failed', result.stdout)
        self.assertEqual(self.destination.read_text(), 'existing user installation')

    def test_authorized_update_replaces_regular_helper(self):
        self.destination.parent.mkdir(parents=True)
        self.destination.write_text('existing user installation')
        result = self.run_install(replace=True)
        self.assertIn('HC_HELPER_INSTALLED', result.stdout)
        self.assertEqual(self.destination.read_bytes(), self.binary.read_bytes())

    def test_even_authorized_update_never_replaces_a_symlink(self):
        self.destination.parent.mkdir(parents=True)
        other=self.home/'other'; other.write_text('preserve me')
        self.destination.symlink_to(other)
        result=self.run_install(replace=True)
        self.assertIn('HC_HELPER_ERROR:install_failed', result.stdout)
        self.assertTrue(self.destination.is_symlink())
        self.assertEqual(other.read_text(),'preserve me')

    def probe(self):
        source=SOURCE.read_text()
        body=re.search(r'static var probeScript.*?#"""(.*?)"""#', source, re.S)[1]
        return subprocess.run(['/bin/sh','-c',textwrap.dedent(body)],env=self.env,capture_output=True,text=True,timeout=10)

    def test_probe_prefers_managed_copy_without_installing_or_replacing(self):
        self.destination.parent.mkdir(parents=True)
        self.destination.write_bytes(self.binary.read_bytes()); self.destination.chmod(0o755)
        external=self.fake/'herdcats-bridge'
        external.write_text('#!/bin/sh\nprintf "old external copy\\n"\n'); external.chmod(0o755)
        before=self.destination.read_bytes()
        result=self.probe()
        self.assertTrue(result.stdout.startswith('HC_HELPER_MANAGED\n'), result.stderr)
        ver = self.report['version']
        self.assertIn(f'"version":"{ver}"', result.stdout)
        self.assertEqual(before,self.destination.read_bytes())

    def test_probe_never_executes_a_symlink_destination(self):
        self.destination.parent.mkdir(parents=True)
        other=self.home/'untrusted'
        other.write_text('#!/bin/sh\ntouch "$HOME/PROBE_EXECUTED"\n'); other.chmod(0o755)
        self.destination.symlink_to(other)
        result=self.probe()
        self.assertEqual(result.stdout,'HC_HELPER_BLOCKED\n')
        self.assertFalse((self.home/'PROBE_EXECUTED').exists())

    def test_probe_failure_is_not_mistaken_for_missing(self):
        self.destination.parent.mkdir(parents=True)
        self.destination.write_text('#!/bin/sh\nexit 1\n'); self.destination.chmod(0o755)
        result=self.probe()
        self.assertIn('HC_HELPER_MANAGED\nHC_HELPER_PROBE_FAILED',result.stdout)
        self.assertEqual(self.destination.read_text(),'#!/bin/sh\nexit 1\n')

if __name__ == '__main__': unittest.main()
