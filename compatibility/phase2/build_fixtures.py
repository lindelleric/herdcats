#!/usr/bin/env python3
"""Build phase-2 evidence from disposable captures; never use real user sessions.

Usage: build_fixtures.py --raw DIR --spike-root PATH
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('phase1_build', ROOT.parent / 'phase1/build_fixtures.py')
phase1 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(phase1)
sanitize = phase1.phase0_sanitize
FORBIDDEN = phase1.FORBIDDEN + ('sk-proj-', 'Bearer ', 'AGENTS.md', '<available_skills>', 'creator_user_id', 'creator_account_id')
KEEP = {'herdcats-probe', 'Size', 'Pick a size', 'Small', 'Large', 'Choose small', 'Choose large',
        'Choose a small size.', 'Choose a large size.', 'Toppings', 'Pick toppings',
        'Pick toppings (select all that apply; enter multiple choices in the free-text field).',
        'Cheese', 'Olives', 'Basil', 'Pepper', 'Cheese, Basil', 'Cheese, Basil, Pepper',
        'user_note: Cheese, Basil', 'Color', 'Pick a color', 'Color: Pick a color',
        'Red (Choose red)', 'Blue (Choose blue)', 'Green'}
KEEP.update(f'Add {label}{period}' for label in ('cheese', 'olives', 'basil') for period in ('', '.'))


def authored(value):
    # Exact allowlist: a prompt mentioning the probe may also contain private context.
    return value in KEEP


def clean(value, key=''):
    if isinstance(value, dict):
        if key == 'answers' and value and all(isinstance(v, dict) and 'answers' in v for v in value.values()):
            return {sanitize(k, 'id'): clean(v) for k, v in value.items()}
        return {k: clean(v, k) for k, v in value.items() if k not in phase1.OMIT}
    if isinstance(value, list):
        return [clean(v, key) for v in value]
    if not isinstance(value, str) or not value:
        return value
    if key == 'text' and value.startswith('<send_user_message_question_reply>\n'):
        replies = json.loads(value.split('\n')[1])
        for reply in replies:
            link = json.loads(reply['questionItemId'])
            link[1] = sanitize(link[1], 'id')
            reply['questionItemId'] = json.dumps(link, separators=(',', ':'))
            reply['answer'] = clean(reply['answer'])
            reply['question'] = clean(reply['question'])
        return '<send_user_message_question_reply>\n' + json.dumps(replies, ensure_ascii=False) + '\n</send_user_message_question_reply>'
    if key in {'arguments', 'output', 'input', 'content'}:
        try:
            return json.dumps(clean(json.loads(value)), ensure_ascii=False)
        except (ValueError, TypeError):
            pass
    if key in phase1.IDENTITIES:
        return sanitize(value, 'id')
    if authored(value):
        return value
    if key == 'name' and value in {'request_user_input', 'request_user_input_async', 'AskUserQuestion', 'sleep'}:
        return value
    return sanitize(value, key)


def records(path, keep):
    return [clean(r) for line in path.read_text().splitlines() if line.strip()
            for r in [json.loads(line)] if keep(r)]


def dump(rows):
    return ''.join(json.dumps(r, ensure_ascii=False) + '\n' for r in rows)


def pending(rows, kind):
    if kind == 'claude':
        call = next(b for r in rows if r['type'] == 'assistant'
                    for b in r['message']['content'] if b.get('name') == 'AskUserQuestion')
        end = next(i for i, r in enumerate(rows) if r['type'] == 'user'
                   and isinstance(r['message']['content'], list)
                   and any(b.get('tool_use_id') == call['id'] for b in r['message']['content']))
    else:
        name = 'request_user_input_async' if kind == 'async' else 'request_user_input'
        call = next(r['payload'] for r in rows if r.get('payload', {}).get('name') == name)
        if kind == 'async':
            accepted = next(i for i, r in enumerate(rows)
                            if r.get('payload', {}).get('type') == 'function_call_output'
                            and r['payload']['call_id'] == call['call_id']
                            and json.loads(r['payload']['output']) == {'accepted': True})
            # The sleep output is produced after the reply has already arrived.
            end = next(i for i in range(accepted + 1, len(rows))
                       if rows[i].get('payload', {}).get('type') == 'function_call_output')
            # Drop trailing completion events too; the pending screen is the sleep call.
            end = max(i + 1 for i in range(accepted + 1, end)
                      if rows[i].get('payload', {}).get('type') == 'function_call')
        else:
            end = next(i for i, r in enumerate(rows) if r.get('payload', {}).get('type') == 'function_call_output'
                       and r['payload']['call_id'] == call['call_id'])
    return rows[:end]


SCREENS = [
    ('claude-question-single', 'claude-q1-screen', '←', 'Esc to cancel'),
    ('claude-question-multi', 'claude-q2-toggled-screen', '←', 'Esc to cancel'),
    ('claude-question-other', 'claude-q2-other-typed-screen', '←', 'Esc to cancel'),
    ('claude-question-review', 'claude-review-screen', '←', '2. Cancel'),
    ('codex-sync-first', 'codex-sync-q1-screen', 'Question 1/2', 'enter to submit answer'),
    ('codex-sync-last', 'codex-sync-after-digit1', 'Question 2/2', 'enter to submit all'),
    ('codex-sync-notes', 'codex-sync-notes', 'Question 2/2', 'enter to submit all'),
    ('codex-async-queued', 'codex-async-queued-screen', 'Queued follow-up inputs', '⇧← to answer'),
    ('codex-async-picker', 'codex-async-picker-screen', 'Queued follow-up inputs', 'main prompt'),
    ('codex-async-other', 'codex-async-other-typed-screen', 'Queued follow-up inputs', 'main prompt'),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--raw', type=Path, required=True)
    parser.add_argument('--spike-root', required=True)
    args = parser.parse_args()
    outputs = {dest + '-screen.txt': phase1.screen(args.raw / (src + '.txt'), start, args.spike_root, end)
               for dest, src, start, end in SCREENS}
    for dest, src, kind, keep in [
        ('claude-question-transcript', 'claude-transcript', 'claude', phase1.keep_claude),
        ('codex-sync-rollout', 'codex-sync-rollout', 'sync', phase1.keep_codex),
        ('codex-async-rollout', 'codex-async-rollout', 'async', phase1.keep_codex),
    ]:
        rows = records(args.raw / (src + '.jsonl'), keep)
        outputs[dest + '.jsonl'] = dump(rows)
        outputs[dest + '-pending.jsonl'] = dump(pending(rows, kind))
    # Validate every output before writing any fixture.
    for name, text in outputs.items():
        for forbidden in FORBIDDEN:
            if forbidden in text:
                raise ValueError(f'{name}: redaction left {forbidden!r}')
    versions = {'claude': '2.1.307', 'codex': '0.163.0'}
    try:
        versions['herdr'] = subprocess.check_output(['herdr', '--version'], text=True).strip().removeprefix('herdr ')
    except (OSError, subprocess.CalledProcessError):
        pass
    files = {}
    (ROOT / 'fixtures').mkdir(exist_ok=True)
    for name, text in outputs.items():
        data = text.encode()
        (ROOT / 'fixtures' / name).write_bytes(data)
        files[name] = {'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
    (ROOT / 'manifest.json').write_text(json.dumps({
        'capture_kind': 'sanitized live disposable probe sessions', 'versions': versions,
        'redaction': 'Exact probe text allowlist; phase 0 pseudonymous IDs and text redaction; phase 1 record filters and anchored screen crops.',
        'files': files,
    }, indent=2) + '\n')


if __name__ == '__main__':
    main()
