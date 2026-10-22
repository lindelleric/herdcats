#!/usr/bin/env python3
"""Build phase-2 evidence from disposable captures; never use real user sessions.

Usage: build_fixtures.py --raw DIR --spike-root PATH
       build_fixtures.py --from-fixtures  # regenerate derived evidence only
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
        'user_note: Cheese, Basil', 'Color', 'Pick a color', 'Pick a color?', 'Red', 'Blue', 'Choose red', 'Choose blue', 'Color: Pick a color',
        'Red (Choose red)', 'Blue (Choose blue)', 'Green', 'None of the above', 'Choose Small.', 'Choose Large.'}
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
        # Live Claude 2.1.307 writes tool_use and tool_result only after the
        # picker completes. Pending therefore ends BEFORE the assistant call.
        end = next(i for i, r in enumerate(rows) if r['type'] == 'assistant'
                   and any(b.get('name') == 'AskUserQuestion' for b in r['message']['content']))
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


def claude_spool(rows):
    """Derived spool evidence: phase-1 hook shape with phase-2 probe input."""
    hooks = [json.loads(line) for line in (ROOT.parent / 'phase1/fixtures/claude-hooks.jsonl').read_text().splitlines()]
    template = next(r for r in hooks if r['event'] == 'PermissionRequest'
                    and r['payload'].get('tool_name') == 'AskUserQuestion')
    payload = dict(template['payload'])
    call = next(b for r in rows if r['type'] == 'assistant'
                for b in r['message']['content'] if b.get('name') == 'AskUserQuestion')
    payload['tool_input'] = call['input']
    payload['session_id'] = rows[0]['sessionId']
    payload['prompt_id'] = next(r['promptId'] for r in rows if r.get('promptId'))
    # Fixed hook time is 1s after the sanitized tool_use (2026-10-13 00:00:00 UTC).
    return [{'v': 1, 't': 1791849601.0, 'agent': 'claude', 'event': 'PermissionRequest',
             'session_id': payload['session_id'], 'pane': 'w1:p1', 'payload': payload}]


SCREENS = [
    ('claude-question-one', 'claude-one-screen', '☐ Color', 'Esc to cancel'),
    ('claude-question-one-multi', 'claude-one-multi-screen', '←', 'Esc to cancel'),
    ('claude-question-one-multi-review', 'claude-one-multi-review', '←', '2. Cancel'),
    ('claude-question-single', 'claude-q1-screen', '←', 'Esc to cancel'),
    ('claude-question-multi', 'claude-q2-toggled-screen', '←', 'Esc to cancel'),
    ('claude-question-other', 'claude-q2-other-typed-screen', '←', 'Esc to cancel'),
    ('claude-question-review', 'claude-review-screen', '←', '2. Cancel'),
    ('codex-sync-first', 'codex-sync-q1-screen', 'Question 1/2', 'enter to submit answer'),
    ('codex-sync-last', 'codex-sync-after-digit1', 'Question 2/2', 'enter to submit all'),
    ('codex-sync-notes-first', 'codex-sync-notes-first-screen', 'Question 1/2', 'enter to submit answer'),
    ('codex-sync-notes', 'codex-sync-notes', 'Question 2/2', 'enter to submit all'),
    ('codex-async-queued-nocountdown', 'codex-async-queued-nocountdown-screen', 'Queued follow-up inputs', '⇧← to answer'),
    ('codex-async-queued', 'codex-async-queued-screen', 'Queued follow-up inputs', '⇧← to answer'),
    ('codex-async-picker', 'codex-async-picker-screen', 'Queued follow-up inputs', 'main prompt'),
    ('codex-async-other', 'codex-async-other-typed-screen', 'Queued follow-up inputs', 'main prompt'),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--raw', type=Path)
    parser.add_argument('--spike-root')
    parser.add_argument('--from-fixtures', action='store_true', help='rebuild derived evidence from sanitized fixtures')
    parser.add_argument('--screens-only', nargs='+', choices=[item[0] for item in SCREENS], help='refresh selected disposable screen captures without rebuilding transcript captures')
    args = parser.parse_args()
    if args.from_fixtures and args.screens_only:
        parser.error('--from-fixtures and --screens-only cannot be combined')
    if args.from_fixtures:
        outputs = {}
    else:
        if args.raw is None or args.spike_root is None:
            parser.error('--raw and --spike-root are required unless --from-fixtures is used')
        outputs = {dest + '-screen.txt': phase1.screen(args.raw / (src + '.txt'), start, args.spike_root, end)
                   for dest, src, start, end in SCREENS if not args.screens_only or dest in args.screens_only}
        if not args.screens_only:
            for dest, src, kind, keep in [
                ('claude-question-one-transcript', 'claude-one-transcript', 'claude', phase1.keep_claude),
                ('claude-question-transcript', 'claude-transcript', 'claude', phase1.keep_claude),
                ('codex-sync-rollout', 'codex-sync-rollout', 'sync', phase1.keep_codex),
                ('codex-async-rollout', 'codex-async-rollout', 'async', phase1.keep_codex),
            ]:
                rows = records(args.raw / (src + '.jsonl'), keep)
                outputs[dest + '.jsonl'] = dump(rows)
                outputs[dest + '-pending.jsonl'] = dump(pending(rows, kind))
    for prefix in ('claude-question', 'claude-question-one'):
        name = prefix + '-transcript.jsonl'
        source = outputs.get(name) if name in outputs else (ROOT / 'fixtures' / name).read_text()
        rows = [json.loads(line) for line in source.splitlines()]
        outputs[prefix + '-transcript-pending.jsonl'] = dump(pending(rows, 'claude'))
        outputs[prefix + '-spool.jsonl'] = dump(claude_spool(rows))
    notes_name = 'codex-sync-notes-screen.txt'
    notes = outputs[notes_name] if notes_name in outputs else (ROOT / 'fixtures' / notes_name).read_text()
    outputs['codex-sync-notes-placeholder-screen.txt'] = notes.replace('› Cheese, Basil', '› Add notes')
    # Validate every output before writing any fixture.
    for name, text in outputs.items():
        for forbidden in FORBIDDEN:
            if forbidden in text:
                raise ValueError(f'{name}: redaction left {forbidden!r}')
    manifest = json.loads((ROOT / 'manifest.json').read_text()) if args.from_fixtures or args.screens_only else {}
    versions = manifest.get('versions', {'claude': '2.1.307', 'codex': '0.163.0'})
    try:
        if not args.from_fixtures and not args.screens_only:
            versions['herdr'] = subprocess.check_output(['herdr', '--version'], text=True).strip().removeprefix('herdr ')
    except (OSError, subprocess.CalledProcessError):
        pass
    files = manifest.get('files', {})
    (ROOT / 'fixtures').mkdir(exist_ok=True)
    for name, text in outputs.items():
        data = text.encode()
        (ROOT / 'fixtures' / name).write_bytes(data)
        files[name] = {'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
        if name.endswith('-spool.jsonl'):
            files[name]['derived'] = 'Phase-1 PermissionRequest hook shape with phase-2 Claude question input, pseudonymous prompt_id and fixed timestamp; not a live spool capture.'
        elif name == 'codex-sync-notes-placeholder-screen.txt':
            files[name]['derived'] = 'Derived from codex-sync-notes-screen.txt by replacing the notes text with Add notes, matching the empty placeholder observed 2026-10-22; not a live screen capture.'
    (ROOT / 'manifest.json').write_text(json.dumps({
        'capture_kind': 'sanitized live disposable probe sessions', 'versions': versions,
        'redaction': 'Exact probe text allowlist; phase 0 pseudonymous IDs and text redaction; phase 1 record filters and anchored screen crops.',
        'files': files,
    }, indent=2) + '\n')


if __name__ == '__main__':
    main()
