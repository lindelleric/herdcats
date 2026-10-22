#!/usr/bin/env python3
"""Verify phase-2 captured evidence offline. Phase 1 is not currently wired into CI."""
import hashlib
import json
import unittest
from pathlib import Path

from build_fixtures import FORBIDDEN

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
        self.assertEqual(len(manifest['files']), 16)
        for name, entry in manifest['files'].items():
            data = (ROOT / 'fixtures' / name).read_bytes()
            self.assertEqual(len(data), entry['bytes'], name)
            self.assertEqual(hashlib.sha256(data).hexdigest(), entry['sha256'], name)
            for forbidden in FORBIDDEN:
                self.assertNotIn(forbidden, data.decode(), name)
        for prefix in ('claude-question-transcript', 'codex-sync-rollout', 'codex-async-rollout'):
            rows, pending = read(prefix + '.jsonl'), read(prefix + '-pending.jsonl')
            self.assertGreater(len(pending), 0)
            self.assertLess(len(pending), len(rows))
            self.assertEqual(pending, rows[:len(pending)])

    def test_claude_shapes_and_resolution(self):
        rows = read('claude-question-transcript.jsonl')
        pending = read('claude-question-transcript-pending.jsonl')
        uses = [b for r in pending if r['type'] == 'assistant'
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
        self.assertEqual(rows[len(pending)]['message']['content'][0], results(rows)[0])
        self.assertEqual(rows[len(pending)]['toolUseResult']['answers'],
                         {'Pick a size': 'Large', 'Pick toppings': 'Cheese, Basil, Pepper'})

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
        # Expiry/skip via task_complete or turn_aborted is inferred, not captured.
        self.assertTrue(any(r.get('payload', {}).get('type') == 'task_complete'
                            and r['payload'].get('turn_id') == turn for r in rows))

    def test_screen_evidence(self):
        expected = {
            'claude-question-single': ['☐ Size', 'Pick a size', '3. Type something.', '4. Chat about this'],
            'claude-question-multi': ['1. [✔] Cheese', '2. [ ] Olives', '3. [✔] Basil', 'Submit'],
            'claude-question-other': ['❯ 4. [✔] Pepper'],
            'claude-question-review': ['Review your answers', '→ Cheese, Basil, Pepper', '1. Submit answers', '2. Cancel'],
            'codex-sync-first': ['Question 1/2 (2 unanswered)', 'None of the above', 'enter to submit answer'],
            'codex-sync-last': ['Question 2/2 (1 unanswered)', 'enter to submit all'],
            'codex-sync-notes': ['› Cheese, Basil', 'tab or esc to clear notes'],
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
