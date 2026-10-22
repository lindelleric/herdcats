#!/usr/bin/env python3
"""Verify phase-2 captured evidence offline. Phase 1 is not currently wired into CI."""
from datetime import datetime
import hashlib
import json
import unittest
from pathlib import Path

from build_fixtures import FORBIDDEN, claude_spool, pending as pending_prefix

ROOT = Path(__file__).resolve().parent


def read(name):
    return [json.loads(line) for line in (ROOT / 'fixtures' / name).read_text().splitlines()]


def call(rows, name):
    return next(r['payload'] for r in rows if r.get('payload', {}).get('name') == name)


def output(rows, request):
    return [json.loads(r['payload']['output']) for r in rows
            if r.get('payload', {}).get('type') == 'function_call_output'
            and r['payload']['call_id'] == request['call_id']]


def replies(rows):
    return [json.loads(b['text'].split('\n')[1]) for r in rows
            if r.get('payload', {}).get('role') == 'user'
            for b in r['payload']['content'] if '<send_user_message_question_reply>' in b.get('text', '')]


class QuestionEvidence(unittest.TestCase):
    def test_manifest_and_redaction(self):
        manifest = json.loads((ROOT / 'manifest.json').read_text())
        self.assertEqual(manifest['versions']['claude'], '2.1.307')
        self.assertEqual(manifest['versions']['codex'], '0.163.0')
        self.assertEqual(set(manifest['files']), {p.name for p in (ROOT / 'fixtures').iterdir()})
        self.assertEqual(len(manifest['files']), 26)
        self.assertIn('not a live screen capture', manifest['files']['codex-sync-notes-placeholder-screen.txt']['derived'])
        for name in ('claude-question-spool.jsonl', 'claude-question-one-spool.jsonl'):
            self.assertIn('not a live spool capture', manifest['files'][name]['derived'])
        for name, entry in manifest['files'].items():
            data = (ROOT / 'fixtures' / name).read_bytes()
            self.assertEqual(len(data), entry['bytes'], name)
            self.assertEqual(hashlib.sha256(data).hexdigest(), entry['sha256'], name)
            for forbidden in FORBIDDEN:
                self.assertNotIn(forbidden, data.decode(), name)
        for prefix in ('claude-question-one-transcript', 'claude-question-transcript', 'codex-sync-rollout', 'codex-async-rollout'):
            rows, pending = read(prefix + '.jsonl'), read(prefix + '-pending.jsonl')
            self.assertGreater(len(pending), 0)
            self.assertLess(len(pending), len(rows))
            self.assertEqual(pending, rows[:len(pending)])

    def test_claude_shapes_and_resolution(self):
        rows = read('claude-question-transcript.jsonl')
        pending = read('claude-question-transcript-pending.jsonl')
        uses = [b for r in rows if r['type'] == 'assistant'
                for b in r['message']['content'] if b.get('name') == 'AskUserQuestion']
        self.assertEqual(len(uses), 1)
        questions = uses[0]['input']['questions']
        self.assertEqual([q['header'] for q in questions], ['Size', 'Toppings'])
        self.assertEqual([q['question'] for q in questions], ['Pick a size', 'Pick toppings'])
        self.assertEqual([q['multiSelect'] for q in questions], [False, True])
        self.assertEqual([[o['label'] for o in q['options']] for q in questions],
                         [['Small', 'Large'], ['Cheese', 'Olives', 'Basil']])
        self.assertTrue(all(isinstance(o['description'], str) for q in questions for o in q['options']))
        def results(records):
            return [b for r in records if r['type'] == 'user' and isinstance(r['message']['content'], list)
                    for b in r['message']['content'] if b.get('tool_use_id') == uses[0]['id']]
        self.assertEqual(results(pending), [])
        self.assertEqual(len(results(rows)), 1)
        self.assertEqual(rows[len(pending)]['type'], 'assistant')
        result = next(r for r in rows if r['type'] == 'user' and isinstance(r['message']['content'], list)
                      and any(b.get('tool_use_id') == uses[0]['id'] for b in r['message']['content']))
        self.assertEqual(result['message']['content'][0], results(rows)[0])
        self.assertEqual(result['toolUseResult']['answers'],
                         {'Pick a size': 'Large', 'Pick toppings': 'Cheese, Basil, Pepper'})

    def test_claude_one_question(self):
        rows = read('claude-question-one-transcript.jsonl')
        pending = read('claude-question-one-transcript-pending.jsonl')
        uses = [b for r in rows if r['type'] == 'assistant'
                for b in r['message']['content'] if b.get('name') == 'AskUserQuestion']
        self.assertEqual(len(uses), 1)
        self.assertEqual(uses[0]['input']['questions'], [{
            'question': 'Pick a color?', 'header': 'Color', 'multiSelect': False,
            'options': [{'label': 'Red', 'description': 'Choose red'},
                        {'label': 'Blue', 'description': 'Choose blue'}]}])
        result = next(r for r in rows if r['type'] == 'user' and isinstance(r['message']['content'], list)
                      and any(b.get('tool_use_id') == uses[0]['id'] for b in r['message']['content']))
        self.assertEqual(result['message']['content'][0]['tool_use_id'], uses[0]['id'])
        self.assertEqual(result['message']['content'][0]['type'], 'tool_result')
        self.assertEqual(result['toolUseResult']['answers'], {'Pick a color?': 'Red'})
        screen = (ROOT / 'fixtures' / 'claude-question-one-screen.txt').read_text()
        self.assertTrue(screen.startswith(' ☐ Color') or screen.startswith('☐ Color'))
        self.assertIn('Pick a color?', screen)
        self.assertIn('Enter to select · ↑/↓ to navigate · Esc to cancel', screen)
        self.assertNotIn('Submit', screen)
        self.assertNotIn('←', screen)

    def test_claude_pending_is_pre_call_and_spool_is_reproducible_derived_evidence(self):
        for prefix in ('claude-question', 'claude-question-one'):
            rows = read(prefix + '-transcript.jsonl')
            pending = read(prefix + '-transcript-pending.jsonl')
            self.assertFalse(any(b.get('name') == 'AskUserQuestion' for r in pending
                                 if r['type'] == 'assistant' for b in r['message']['content']))
            self.assertEqual(pending, pending_prefix(rows, 'claude'))
            self.assertEqual(rows[len(pending)]['type'], 'assistant')
            spool = read(prefix + '-spool.jsonl')
            self.assertEqual(spool, claude_spool(rows))
            self.assertEqual(len(spool), 1)
            record = spool[0]
            self.assertEqual(record['v'], 1)
            self.assertEqual(record['t'], 1791849601.0)
            self.assertEqual(record['agent'], 'claude')
            self.assertEqual(record['event'], 'PermissionRequest')
            self.assertEqual(record['session_id'], rows[0]['sessionId'])
            self.assertEqual(record['payload']['session_id'], record['session_id'])
            self.assertEqual(record['payload']['prompt_id'], next(r['promptId'] for r in rows if r.get('promptId')))
            call = next(b for r in rows if r['type'] == 'assistant'
                        for b in r['message']['content'] if b.get('name') == 'AskUserQuestion')
            self.assertEqual(record['payload']['tool_input'], call['input'])
            call_record = next(r for r in rows if r['type'] == 'assistant'
                               and any(b.get('name') == 'AskUserQuestion' for b in r['message']['content']))
            self.assertEqual(record['t'] - datetime.fromisoformat(call_record['timestamp']).timestamp(), 1.0)

    def test_codex_sync_shapes_notes_and_resolution(self):
        rows = read('codex-sync-rollout.jsonl')
        pending = read('codex-sync-rollout-pending.jsonl')
        request = call(pending, 'request_user_input')
        questions = json.loads(request['arguments'])['questions']
        self.assertEqual([q['header'] for q in questions], ['Size', 'Toppings'])
        self.assertEqual(len({q['id'] for q in questions}), 2)
        self.assertTrue(all('multiSelect' not in q for q in questions))
        self.assertEqual([[o['label'] for o in q['options']] for q in questions],
                         [['Small', 'Large'], ['Cheese', 'Olives', 'Basil']])
        self.assertTrue(all(isinstance(o['description'], str) for q in questions for o in q['options']))
        self.assertEqual(output(pending, request), [])
        answers = output(rows, request)
        self.assertEqual(answers, [{'answers': {questions[0]['id']: {'answers': ['Small']},
                                               questions[1]['id']: {'answers': ['Cheese', 'user_note: Cheese, Basil']}}}])
        # Question IDs are pseudonymized consistently with answer-map keys.
        self.assertEqual(rows[len(pending)]['payload']['type'], 'function_call_output')
        self.assertEqual(rows[len(pending)]['payload']['call_id'], request['call_id'])

    def test_async_acceptance_is_not_resolution_and_reply_links_call(self):
        rows = read('codex-async-rollout.jsonl')
        pending = read('codex-async-rollout-pending.jsonl')
        self.assertTrue(output(pending, call(pending, 'request_user_input')))
        request = call(pending, 'request_user_input_async')
        self.assertEqual(json.loads(request['arguments']), {'questions': [
            {'title': 'Color: Pick a color', 'options': ['Red (Choose red)', 'Blue (Choose blue)']}]})
        self.assertEqual(output(pending, request), [{'accepted': True}])
        self.assertEqual(replies(pending), [])
        reply = replies(rows)
        self.assertEqual(len(reply), 1)
        self.assertEqual(len(reply[0]), 1)
        self.assertEqual(reply[0][0]['answer'], 'Green')
        self.assertEqual(json.loads(reply[0][0]['questionItemId']),
                         ['request_user_input_async', request['call_id'], 0])
        self.assertEqual(reply[0][0]['question'], json.loads(request['arguments'])['questions'][0]['title'])
        self.assertEqual(pending[-1]['payload']['type'], 'function_call')
        self.assertEqual(pending[-1]['payload']['name'], 'sleep')
        self.assertEqual(output(pending, pending[-1]['payload']), [])
        later_output = next(r['payload'] for r in rows[len(pending):]
                            if r.get('payload', {}).get('type') == 'function_call_output')
        self.assertEqual(later_output['call_id'], pending[-1]['payload']['call_id'])
        accepted_index = next(i for i, r in enumerate(pending)
                              if r.get('payload', {}).get('type') == 'function_call_output'
                              and r['payload']['call_id'] == request['call_id'])
        self.assertFalse(any(r.get('payload', {}).get('type') == 'function_call_output'
                             for r in pending[accepted_index + 1:]))
        turn = request['internal_chat_message_metadata_passthrough']['turn_id']
        self.assertFalse(any(r.get('payload', {}).get('type') in {'task_complete', 'turn_aborted'}
                             and r['payload'].get('turn_id') == turn for r in pending))
        # These captures show an answered async call. Live skip (observed 2026-10-22)
        # writes no reply and remains pending until turn end; expiry remains inferred.
        self.assertTrue(any(r.get('payload', {}).get('type') == 'task_complete'
                            and r['payload'].get('turn_id') == turn for r in rows))

    def test_codex_notes_placeholder_is_derived_from_notes_capture(self):
        notes = (ROOT / 'fixtures/codex-sync-notes-screen.txt').read_text()
        placeholder = (ROOT / 'fixtures/codex-sync-notes-placeholder-screen.txt').read_text()
        self.assertEqual(placeholder, notes.replace('› Cheese, Basil', '› Add notes'))
        self.assertIn('› Add notes', placeholder)

    def test_screen_evidence(self):
        expected = {
            'claude-question-single': ['☐ Size', 'Pick a size', '3. Type something.', '4. Chat about this'],
            'claude-question-one-multi': ['←  ☐ Toppings  ✔ Submit  →', '1. [ ] Cheese', 'Type something', 'Submit', 'Chat about this', '↑/↓ to navigate'],
            'claude-question-one-multi-review': ['←  ☒ Toppings  ✔ Submit  →', 'Review your answers', '● Pick toppings', '→ Cheese, Basil', '1. Submit answers', '2. Cancel'],
            'claude-question-multi': ['1. [✔] Cheese', '2. [ ] Olives', '3. [✔] Basil', 'Submit'],
            'claude-question-other': ['❯ 4. [✔] Pepper'],
            'claude-question-review': ['Review your answers', '→ Cheese, Basil, Pepper', '1. Submit answers', '2. Cancel'],
            'codex-sync-first': ['Question 1/2 (2 unanswered)', 'None of the above', 'enter to submit answer'],
            'codex-sync-last': ['Question 2/2 (1 unanswered)', 'enter to submit all'],
            'codex-sync-notes-first': ['Question 1/2 (2 unanswered)', 'Pick a size', 'Small', 'Large', '› 3. None of the above', '› Add notes', 'tab or esc to clear notes | enter to submit answer'],
            'codex-sync-notes': ['› Cheese, Basil', 'tab or esc to clear notes'],
            'codex-sync-notes-placeholder': ['› Add notes', 'tab or esc to clear notes'],
            'codex-async-queued-nocountdown': ['? 1 question', '⇧← to answer'],
            'codex-async-queued': ['? 1 question · 9s', '⇧← to answer'],
            'codex-async-picker': ['Color: Pick a color', '› 1. Red (Choose red)', '3. Other', '⌃] skip'],
            'codex-async-other': ['› 3. Green', 'enter submit', '⇧→ main prompt'],
        }
        for name, anchors in expected.items():
            text = (ROOT / 'fixtures' / (name + '-screen.txt')).read_text()
            for anchor in anchors:
                self.assertIn(anchor, text, name)


if __name__ == '__main__':
    unittest.main()
