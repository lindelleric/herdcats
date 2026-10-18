#!/usr/bin/env python3
"""Install an explicitly selected, locally built bridge. Never starts a service."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import selectors
import stat
import time
import subprocess
import tempfile
import sys


def probe(binary):
    output = {'stdout': bytearray(), 'stderr': bytearray()}
    deadline = time.monotonic() + 5
    with subprocess.Popen([str(binary), '--version-json'], stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE) as process:
        try:
            with selectors.DefaultSelector() as selector:
                selector.register(process.stdout, selectors.EVENT_READ, 'stdout')
                selector.register(process.stderr, selectors.EVENT_READ, 'stderr')
                while selector.get_map():
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        raise ValueError('version probe timed out')
                    for key, _ in selector.select(remaining):
                        data = os.read(key.fileobj.fileno(), 8193)
                        if not data:
                            selector.unregister(key.fileobj)
                            continue
                        buffer = output[key.data]
                        if len(buffer) + len(data) > 8192:
                            raise ValueError('version probe output too large')
                        buffer.extend(data)
                if process.wait(timeout=max(0, deadline - time.monotonic())) != 0:
                    raise ValueError('version probe failed')
        finally:
            if process.poll() is None:
                process.kill()
            process.wait()
    value = json.loads(output['stdout'])
    if (not isinstance(value, dict) or value.get('name') != 'herdcats-bridge'
            or type(value.get('protocol')) is not int or value['protocol'] != 1
            or value.get('herdr_protocol') != 22
            or not isinstance(value.get('version'), str) or not value['version']):
        raise ValueError('incompatible bridge')
    return value


def check_destination(destination, replace):
    try:
        mode = destination.lstat().st_mode
    except FileNotFoundError:
        return
    if not stat.S_ISREG(mode):
        raise ValueError('destination is not a regular file')
    if not replace:
        raise ValueError('destination exists; use --replace to update explicitly')


def install(binary, bin_dir, replace):
    # Probe the exact bytes to install, not a source that can change afterward.
    with tempfile.TemporaryDirectory(prefix='herdcats-bridge-install-') as scratch:
        checked = Path(scratch) / 'herdcats-bridge'
        shutil.copyfile(binary, checked)
        checked.chmod(0o700)
        version = probe(checked)
        digest = hashlib.sha256(checked.read_bytes()).hexdigest()
        destination = bin_dir / 'herdcats-bridge'
        check_destination(destination, replace)
        bin_dir.mkdir(parents=True, exist_ok=True)
        fd, staged_name = tempfile.mkstemp(prefix='.herdcats-bridge-', dir=bin_dir)
        staged = Path(staged_name)
        try:
            with os.fdopen(fd, 'wb') as output, checked.open('rb') as source:
                shutil.copyfileobj(source, output)
                output.flush()
                os.fchmod(output.fileno(), 0o755)
                os.fsync(output.fileno())
            if replace:
                check_destination(destination, replace=True)
                os.replace(staged, destination)
            else:
                # An existing destination created after the check cannot be overwritten.
                os.link(staged, destination)
            return {'path': str(destination), 'sha256': digest, **version}
        finally:
            staged.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True,
                        help='Trusted binary built for this host; it will be executed to check compatibility')
    parser.add_argument('--bin-dir', type=Path, required=True,
                        help='Explicit installation directory, normally ~/.local/bin')
    parser.add_argument('--replace', action='store_true', help='Explicitly replace an existing regular installation')
    args = parser.parse_args()
    try:
        result = install(args.binary.resolve(strict=True), args.bin_dir.expanduser().absolute(), args.replace)
    except (OSError, ValueError, subprocess.SubprocessError):
        print('bridge_install_failed: check the binary, protocol and destination; existing files require --replace', file=sys.stderr)
        return 1

    print(json.dumps(result))
    return 0


if __name__ == '__main__':
    sys.exit(main())
