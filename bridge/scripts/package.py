#!/usr/bin/env python3
"""Package a native release binary with its per-platform checksum."""
import argparse
import hashlib
import json
import re
from pathlib import Path
import shutil
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--platform', choices=['darwin-arm64', 'darwin-x86_64', 'linux-arm64', 'linux-x86_64'], required=True)
    parser.add_argument('--version', required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", args.version):
        raise SystemExit("Invalid release version")
    info = json.loads(subprocess.check_output([str(args.binary.resolve()), '--version-json'], timeout=5))
    if info.get('version') != args.version or info.get('protocol') != 1:
        raise SystemExit('Package version/protocol mismatch')
    args.out.mkdir(parents=True, exist_ok=True)
    asset = args.out / ('herdcats-bridge-' + args.platform)
    shutil.copyfile(args.binary, asset)
    asset.chmod(0o755)
    digest = hashlib.sha256(asset.read_bytes()).hexdigest()
    installer = Path(__file__).with_name('install.sh').read_text()
    installer, count = re.subn(r'^HB_VERSION=.*$', 'HB_VERSION=' + args.version, installer, flags=re.M)
    if count != 1:
        raise SystemExit('Missing installer version default')
    (args.out / 'install.sh').write_text(installer)
    (args.out / ('SHA256SUMS-' + args.platform)).write_text(digest + '  ' + asset.name + '\n')


if __name__ == '__main__':
    main()
