#!/usr/bin/env python3
"""Build redacted interaction fixtures from an explicitly supplied disposable capture directory.

Usage: python3 build_fixtures.py --raw <capture dir> --spike-root <disposable project root>
Only strings the probe itself authored (prompts, commands, question labels) survive verbatim;
everything else follows the phase 0 redaction rules. Never point this at real user sessions.
"""
import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT.parent / 'phase0'))
from sanitize import IDENTITIES, OMIT, sanitize as phase0_sanitize  # noqa: E402

FIXTURES = ROOT / 'fixtures'
FORBIDDEN = ('/Users/', '/private/', 'zhiyao')
# Probe-authored text: kept verbatim so adapters can be tested against real labels.
MARKERS = ('herdcats-probe', 'Pick a color', 'Pick toppings', 'Pick a size', 'probe.txt', 'probe2.txt')
KEEP = {'Red', 'Blue', 'Small', 'Large', 'Cheese', 'Olives', 'Basil', 'Cheese, Basil',
        'Choose red', 'Choose blue', 'Add cheese', 'Add olives', 'Add basil', 'Color', 'Toppings',
        'Claude needs your permission', 'session', 'default', 'on-request', 'user', 'interrupted'}
KEEP_KEYS = {'tool_name', 'name', 'notification_type', 'permission_mode', 'destination', 'event',
             'agent', 'level', 'delivery', 'HERDR_PANE_ID', 'HERDR_TAB_ID', 'HERDR_WORKSPACE_ID', 'HERDR_ENV'}
PATH_KEYS = {'directories'}
# Correlates a Claude PermissionRequest (which has no tool_use_id) with its PreToolUse.
EXTRA_IDS = {'prompt_id'}

def authored(value):
    return value in KEEP or (any(m in value for m in MARKERS) and not any(f in value for f in FORBIDDEN))

def clean(value, key=''):
    if isinstance(value, dict):
        return {k: clean(v, k) for k, v in value.items() if k not in OMIT}
    if isinstance(value, list):
        return [clean(v, key) for v in value]
    if not isinstance(value, str) or not value:
        return value
    if key in IDENTITIES or key in EXTRA_IDS:
        return phase0_sanitize(value, 'id')
    if key in PATH_KEYS:
        return '/fixture/work'
    if key in KEEP_KEYS and re.fullmatch(r'[A-Za-z0-9_.:-]{1,64}', value):
        return value
    if key in {'input', 'arguments', 'output', 'content'}:
        try:
            return json.dumps(clean(json.loads(value)), ensure_ascii=False)
        except (ValueError, TypeError):
            pass
    if authored(value):
        # Authored replies can embed raw call IDs (Codex questionItemId); pseudonymize them
        # with the same namespace so question and answer stay linked.
        return re.sub(r'call_[0-9a-f]{16,}', lambda m: phase0_sanitize(m.group(0), 'id'), value)
    return phase0_sanitize(value, key)

def keep_claude(record):
    return record.get('type') in {'user', 'assistant', 'permission-mode'}

def keep_codex(record):
    payload = record.get('payload') or {}
    if record.get('type') == 'session_meta':
        return True
    if record.get('type') == 'response_item':
        return payload.get('role') != 'developer'
    return record.get('type') == 'event_msg' and payload.get('type') in {
        'task_started', 'task_complete', 'turn_aborted', 'item_completed'}

def jsonl(source, keep=lambda _: True):
    rows = [json.loads(line) for line in source.read_text().splitlines() if line.strip()]
    return ''.join(json.dumps(clean(r), ensure_ascii=False) + '\n' for r in rows if keep(r))

def mask_path(text, path):
    # Replace the path even where the terminal wrapped it, preserving every newline and indent
    # so wrapped-label parsing stays testable.
    pattern = r'(?:\n\s*)?'.join(re.escape(c) for c in path)
    fake = ('/fixture/work/' + 'x' * len(path))[:len(path)]
    def replace(match):
        chars, i = [], 0
        for c in match.group(0):
            if c.isspace():  # the path itself has no spaces; these are the wrap and indent
                chars.append(c)
            else:
                chars.append(fake[i])
                i += 1
        return ''.join(chars)
    return re.sub(pattern, replace, text)

def screen(source, anchor, spike_root):
    lines = source.read_text().splitlines()
    start = next(i for i, line in enumerate(lines) if anchor in line)
    text = '\n'.join(line.rstrip() for line in lines[start:]).strip('\n') + '\n'
    return mask_path(text, spike_root)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--raw', type=Path, required=True)
    parser.add_argument('--spike-root', required=True)
    args = parser.parse_args()
    raw, root = args.raw, args.spike_root.rstrip('/')
    outputs = {
        'claude-hooks.jsonl': jsonl(raw / 'claude-hooks.jsonl'),
        'claude-transcript.jsonl': jsonl(raw / 'claude-session-transcript.jsonl', keep_claude),
        'claude-approval-screen.txt': screen(raw / 'claude-approval-screen.txt', 'Bash command', root + '/claude'),
        'claude-question-screen.txt': screen(raw / 'claude-question-screen.txt', '☐ Color', root + '/claude'),
        'codex-hooks.jsonl': jsonl(raw / 'codex-hooks.jsonl'),
        'codex-pending-rollout.jsonl': jsonl(raw / 'codex-pending-rollout.jsonl', keep_codex),
        'codex-rollout.jsonl': jsonl(raw / 'codex-session-rollout.jsonl', keep_codex),
        'codex-approval-screen.txt': screen(raw / 'codex-approval-screen.txt', 'Would you like to run', root + '/codex'),
        'codex-question-screen.txt': screen(raw / 'codex-question-screen.txt', 'Pick a color', root + '/codex'),
        'codex-question-picker-screen.txt': screen(raw / 'codex-question-picker-screen.txt', 'Queued follow-up', root + '/codexq'),
        'codex-question-answer-rollout.jsonl': jsonl(raw / 'codex-question-answer-rollout.jsonl', keep_codex),
    }
    FIXTURES.mkdir(exist_ok=True)
    files = {}
    for name, text in outputs.items():
        for forbidden in FORBIDDEN:
            if forbidden in text:
                sys.exit(f'{name}: redaction left {forbidden!r}')
        (FIXTURES / name).write_text(text)
        data = text.encode()
        files[name] = {'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
    manifest = {
        'capture_kind': 'sanitized live disposable sessions (herdr --session, temp project, probe-only prompts)',
        'versions': {'herdr': '0.9.3', 'protocol': 22, 'claude': '2.1.301', 'codex': '0.162.1'},
        'redaction': 'Probe-authored text kept verbatim; all other free text replaced; IDs pseudonymized with the phase 0 namespace; paths masked in place.',
        'files': files,
    }
    (FIXTURES / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')

if __name__ == '__main__':
    main()
