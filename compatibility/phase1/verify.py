#!/usr/bin/env python3
"""Verify phase 1 interaction evidence without running agents or using a network."""
import hashlib
import json
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parent
FIXTURES = ROOT / 'fixtures'

def read(name):
    text = (FIXTURES / name).read_text()
    if name.endswith('.jsonl'):
        return [json.loads(line) for line in text.splitlines()]
    return text if name.endswith('.txt') else json.loads(text)

def hooks(name, event, tool=None):
    return [r['payload'] for r in read(name)
            if r['event'] == event and (tool is None or r['payload'].get('tool_name') == tool)]

def options(screen):
    """Numbered dialog options, joining labels the terminal wrapped onto indented lines."""
    found, current = [], None
    for line in screen.splitlines():
        match = re.match(r'^\s*[❯›]?\s*(\d+)\.\s+(.*)$', line)
        if match:
            current = [int(match.group(1)), match.group(2).strip()]
            found.append(current)
        elif current and line.startswith('      ') and line.strip():
            current[1] += line.strip()
        else:
            current = None
    return [(n, label) for n, label in found]

class InteractionEvidence(unittest.TestCase):
    def test_fixture_manifest_and_redaction(self):
        manifest = read('manifest.json')
        self.assertEqual(set(manifest['files']), {p.name for p in FIXTURES.iterdir()} - {'manifest.json'})
        for name, entry in manifest['files'].items():
            data = (FIXTURES / name).read_bytes()
            self.assertEqual(len(data), entry['bytes'], name)
            self.assertEqual(hashlib.sha256(data).hexdigest(), entry['sha256'], name)
            for forbidden in ['/Users/', '/private/', 'zhiyao', 'sk-proj-', 'Bearer ']:
                self.assertNotIn(forbidden, data.decode(), name)

    def test_claude_permission_request_correlates_through_pre_tool_use(self):
        pre = hooks('claude-hooks.jsonl', 'PreToolUse', 'Bash')[0]
        request = hooks('claude-hooks.jsonl', 'PermissionRequest', 'Bash')[0]
        self.assertNotIn('tool_use_id', request)
        self.assertEqual(request['prompt_id'], pre['prompt_id'])
        self.assertEqual(request['tool_input']['command'], pre['tool_input']['command'])
        self.assertEqual(request['permission_suggestions'][0]['type'], 'addDirectories')
        uses = [b['id'] for r in read('claude-transcript.jsonl') if r['type'] == 'assistant'
                for b in r['message']['content'] if b.get('type') == 'tool_use']
        self.assertIn(pre['tool_use_id'], uses)

    def test_claude_approval_screen_mirrors_terminal_options(self):
        parsed = options(read('claude-approval-screen.txt'))
        self.assertEqual([n for n, _ in parsed], [1, 2, 3, 4])
        self.assertEqual(parsed[0][1], 'Yes')
        self.assertTrue(parsed[1][1].startswith('Yes, and always allow access to /fixture/work/'))
        self.assertTrue(parsed[1][1].endswith('from this project'))
        self.assertEqual(parsed[3][1], 'No')

    def test_claude_question_is_a_permission_request_with_structured_answers(self):
        request = hooks('claude-hooks.jsonl', 'PermissionRequest', 'AskUserQuestion')[0]
        questions = request['tool_input']['questions']
        self.assertEqual([q['multiSelect'] for q in questions], [False, True])
        answered = [r['toolUseResult']['answers'] for r in read('claude-transcript.jsonl')
                    if isinstance(r.get('toolUseResult'), dict) and 'answers' in r['toolUseResult']]
        self.assertEqual(answered, [{'Pick a color': 'Red', 'Pick toppings': 'Cheese, Basil'}])
        self.assertIn('☐ Color  ☐ Toppings  ✔ Submit', read('claude-question-screen.txt'))

    def test_codex_permission_request_has_no_call_id_but_matches_pre_tool_use(self):
        request = hooks('codex-hooks.jsonl', 'PermissionRequest')[0]
        self.assertNotIn('tool_use_id', request)
        pres = [p for p in hooks('codex-hooks.jsonl', 'PreToolUse', 'Bash')
                if p['tool_input']['command'] == request['tool_input']['command']]
        self.assertTrue(pres)
        self.assertEqual(request['turn_id'], pres[-1]['turn_id'])

    def test_codex_pending_escalation_is_code_mode_javascript(self):
        rows = read('codex-pending-rollout.jsonl')
        calls = [r['payload'] for r in rows if r['payload'].get('type') == 'custom_tool_call']
        outputs = {r['payload']['call_id'] for r in rows if r['payload'].get('type') == 'custom_tool_call_output'}
        pending = [c for c in calls if c['call_id'] not in outputs]
        self.assertEqual(len(pending), 1)
        self.assertEqual(pending[0]['name'], 'exec')
        self.assertIn('require_escalated', pending[0]['input'])

    def test_codex_approval_options_carry_letter_shortcuts(self):
        parsed = options(read('codex-approval-screen.txt'))
        self.assertEqual([label.rsplit(' ', 1)[-1] for _, label in parsed], ['(y)', '(p)', '(esc)'])

    def test_codex_question_is_async_and_never_a_permission_request(self):
        pre = hooks('codex-hooks.jsonl', 'PreToolUse', 'request_user_input_async')[0]
        self.assertEqual([q['options'] for q in pre['tool_input']['questions']], [['Red', 'Blue'], ['Small', 'Large']])
        self.assertFalse(hooks('codex-hooks.jsonl', 'PermissionRequest', 'request_user_input_async'))
        outputs = [json.loads(r['payload']['output']) for r in read('codex-rollout.jsonl')
                   if r['payload'].get('type') == 'function_call_output' and r['payload']['call_id'] == pre['tool_use_id']]
        self.assertEqual(outputs, [{'accepted': True}])

    def test_codex_question_reply_links_back_to_the_call(self):
        rows = read('codex-question-answer-rollout.jsonl')
        ask = next(r['payload'] for r in rows if r['payload'].get('name') == 'request_user_input_async')
        texts = [c['text'] for r in rows if r['payload'].get('role') == 'user'
                 for c in r['payload']['content'] if '<send_user_message_question_reply>' in c.get('text', '')]
        self.assertEqual(len(texts), 1)
        reply = json.loads(texts[0].split('\n')[1])
        self.assertEqual(reply[0]['answer'], 'Blue')
        self.assertEqual(json.loads(reply[0]['questionItemId']), ['request_user_input_async', ask['call_id'], 0])
        asked = [r['payload']['item'] for r in rows if r['payload'].get('type') == 'item_completed'
                 and r['payload']['item'].get('delivery') == 'async']
        self.assertEqual(asked[0]['questions'], [{'title': 'Pick a color', 'options': ['Red', 'Blue']}])

    def test_codex_question_picker_offers_other_and_skip(self):
        screen = read('codex-question-picker-screen.txt')
        self.assertEqual([label for _, label in options(screen)], ['Red', 'Blue', 'Other'])
        self.assertIn('⌃] skip', screen)

class AgentRoadmap(unittest.TestCase):
    def test_roadmap_covers_herdr_agents_and_gates_on_identity(self):
        roadmap = json.loads((ROOT / 'agent-roadmap.json').read_text())
        agents = {a['id']: a for a in roadmap['agents']}
        self.assertEqual(len(agents), len(roadmap['agents']))
        self.assertEqual(sum(a['tier'] == 'integration' for a in agents.values()), 18)
        self.assertEqual(roadmap['order'][:2], ['claude', 'codex'])
        self.assertTrue(set(roadmap['order']) <= set(agents))
        for agent in agents.values():
            if agent['tier'] == 'screen_only':
                self.assertEqual(agent['status'], 'not_possible', agent['id'])

if __name__ == '__main__':
    unittest.main()
