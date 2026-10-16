"""Execute the actual Swift-embedded SSH launcher on a controlled local host."""
import os
from pathlib import Path
import re
import subprocess
import tempfile
import textwrap
import unittest

SOURCE = Path(__file__).resolve().parents[2] / 'ios/Herdcats/Networking/AgentBridgeClient.swift'

class MobileLauncherTests(unittest.TestCase):
    def script(self):
        body = re.search(r'let script = """(.*?)"""', SOURCE.read_text(), re.S)[1]
        return textwrap.dedent(body).replace('\\\\', '\\')

    def test_managed_installation_wins_over_old_path_binary(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            managed = home / '.local/bin/herdcats-bridge'
            old = home / 'old/herdcats-bridge'
            for path, label in [(managed, 'managed'), (old, 'old')]:
                path.parent.mkdir(parents=True)
                path.write_text('#!/bin/sh\nprintf "%s\\n" ' + label + '\n')
                path.chmod(0o755)
            result = subprocess.run(['/bin/sh', '-c', self.script()], input='{}\n', text=True,
                env={**os.environ, 'HOME': str(home), 'PATH': str(old.parent) + ':/usr/bin:/bin'}, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, 'managed\n')

    def test_missing_helper_emits_one_decodable_bootstrap_frame(self):
        with tempfile.TemporaryDirectory() as tmp:
            result = subprocess.run(['/bin/sh', '-c', self.script()], input='{}\n', text=True,
                env={**os.environ, 'HOME': tmp, 'PATH': '/usr/bin:/bin'}, capture_output=True)
            self.assertEqual(result.returncode, 127)
            self.assertEqual(result.stdout, '{"error":{"code":"helper_missing"}}\n')
            self.assertEqual(result.stderr, '')

if __name__ == '__main__':
    unittest.main()
