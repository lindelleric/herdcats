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


def configure_codex_daemon(disable_daemon=False, is_interactive=False):
    """
    Opt-in only (Decision Q21):
    If stdin is a TTY, ask (default No) whether to set features.daemon_auto_start = false
    in $CODEX_HOME/config.toml (else ~/.codex/config.toml), explaining that Codex's shared daemon
    hides which terminal a session belongs to, so Herdcats cannot identify Codex panes without it.
    Non-interactive: only with an explicit --codex-disable-daemon flag; never silently.
    Preserve all other content, write a timestamped backup first, idempotent, no-op if Codex is not installed.
    """
    codex_home = os.environ.get('CODEX_HOME')
    if codex_home:
        codex_dir = Path(codex_home)
    else:
        home = os.environ.get('HOME')
        if not home:
            return
        codex_dir = Path(home) / '.codex'

    # No-op if Codex is not installed
    if not codex_dir.is_dir() and not shutil.which('codex'):
        return

    should_disable = False
    if disable_daemon:
        should_disable = True
    elif is_interactive:
        sys.stderr.write(
            "\nCodex's shared daemon hides which terminal a session belongs to, "
            "so Herdcats cannot identify Codex panes without disabling it.\n"
            f"Disable Codex shared daemon (set features.daemon_auto_start = false in {codex_dir / 'config.toml'})? [y/N]: "
        )
        sys.stderr.flush()
        try:
            choice = sys.stdin.readline().strip().lower()
            if choice in ('y', 'yes'):
                should_disable = True
        except Exception:
            should_disable = False

    if not should_disable:
        return

    config_path = codex_dir / 'config.toml'
    if config_path.is_file():
        content = config_path.read_text(encoding='utf-8')
    else:
        content = ''

    # Check if already false (idempotent)
    lines = content.splitlines(keepends=True)
    in_features = False
    already_false = False
    has_features_section = False
    daemon_line_idx = None

    for i, line in enumerate(lines):
        stripped = line.strip()
        if stripped.startswith('#'):
            continue
        if stripped.startswith('[') and stripped.endswith(']'):
            table_name = stripped[1:-1].strip()
            in_features = (table_name == 'features')
            if in_features:
                has_features_section = True
        elif in_features and stripped.startswith('daemon_auto_start'):
            parts = stripped.split('=', 1)
            if len(parts) == 2:
                val = parts[1].strip()
                if val == 'false':
                    already_false = True
                daemon_line_idx = i
        elif stripped.startswith('features.daemon_auto_start'):
            parts = stripped.split('=', 1)
            if len(parts) == 2:
                val = parts[1].strip()
                if val == 'false':
                    already_false = True
                daemon_line_idx = i

    if already_false:
        return

    codex_dir.mkdir(parents=True, exist_ok=True)
    if config_path.is_file():
        timestamp = time.strftime('%Y%m%d%H%M%S')
        backup_path = config_path.with_name(f"{config_path.name}.bak.{timestamp}")
        counter = 0
        while backup_path.exists():
            counter += 1
            backup_path = config_path.with_name(f"{config_path.name}.bak.{timestamp}_{counter}")
        shutil.copy2(config_path, backup_path)

    if daemon_line_idx is not None:
        lines[daemon_line_idx] = 'daemon_auto_start = false\n'
        new_content = ''.join(lines)
    elif has_features_section:
        new_lines = []
        for line in lines:
            new_lines.append(line)
            stripped = line.strip()
            if stripped.startswith('[') and stripped.endswith(']') and stripped[1:-1].strip() == 'features':
                new_lines.append('daemon_auto_start = false\n')
        new_content = ''.join(new_lines)
    else:
        prefix = '' if not content or content.endswith('\n\n') else ('\n' if content.endswith('\n') else '\n\n')
        new_content = content + prefix + '[features]\ndaemon_auto_start = false\n'

    fd, tmp_name = tempfile.mkstemp(prefix='.config.toml-', dir=codex_dir)
    try:
        with os.fdopen(fd, 'w', encoding='utf-8') as f:
            f.write(new_content)
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp_name, config_path)
    finally:
        Path(tmp_name).unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True,
                        help='Trusted binary built for this host; it will be executed to check compatibility')
    parser.add_argument('--bin-dir', type=Path, required=True,
                        help='Explicit installation directory, normally ~/.local/bin')
    parser.add_argument('--replace', action='store_true', help='Explicitly replace an existing regular installation')
    parser.add_argument('--codex-disable-daemon', action='store_true',
                        help='Opt in to disabling Codex daemon_auto_start in Codex config')
    args = parser.parse_args()
    try:
        result = install(args.binary.resolve(strict=True), args.bin_dir.expanduser().absolute(), args.replace)
        configure_codex_daemon(disable_daemon=args.codex_disable_daemon, is_interactive=sys.stdin.isatty())
    except (OSError, ValueError, subprocess.SubprocessError):
        print('bridge_install_failed: check the binary, protocol and destination; existing files require --replace', file=sys.stderr)
        return 1
    print(json.dumps(result))
    return 0


if __name__ == '__main__':
    sys.exit(main())
