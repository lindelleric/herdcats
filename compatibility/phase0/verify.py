#!/usr/bin/env python3
"""Verify captured compatibility evidence without running agents or using a network."""
import hashlib
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parent
FIXTURES = ROOT / 'fixtures'

def read(name):
    text = (FIXTURES / name).read_text()
    return [json.loads(line) for line in text.splitlines()] if name.endswith('.jsonl') else json.loads(text)

def agent(trace, key):
    return trace[key]['result']['agent']

def reference(entry):
    return entry.get('agent_session', {}).get('value')

class CompatibilityEvidence(unittest.TestCase):
    def test_fixture_manifest_and_redaction(self):
        manifest = read('manifest.json')
        self.assertEqual(manifest['versions']['protocol'], 22)
        self.assertEqual(set(manifest['files']), {p.name for p in FIXTURES.iterdir() if p.is_file()} - {'manifest.json'})
        for name, entry in manifest['files'].items():
            data = (FIXTURES / name).read_bytes()
            self.assertEqual(len(data), entry['bytes'], name)
            self.assertEqual(hashlib.sha256(data).hexdigest(), entry['sha256'], name)
            text = data.decode('utf-8')
            for forbidden in ['/Users/', '/private/tmp/', 'zhiyao', 'sk-proj-', 'Bearer ']:
                self.assertNotIn(forbidden, text, name)
            read(name)

    def test_claude_resume_move_shared_cwd_and_replacement(self):
        trace = read('session-identity-trace.json')
        first = agent(trace, 'claude-agent')
        resumed = agent(trace, 'claude-resumed')
        moved = agent(trace, 'claude-after-move')
        second = agent(trace, 'claude-second')
        replacement = agent(trace, 'claude-replacement')
        self.assertTrue(reference(first))
        self.assertEqual(reference(first), reference(resumed))
        self.assertEqual(reference(resumed), reference(moved))
        self.assertNotEqual(resumed['pane_id'], moved['pane_id'])
        self.assertEqual(resumed['terminal_id'], moved['terminal_id'])
        self.assertEqual(first['cwd'], second['cwd'])
        self.assertNotEqual(reference(first), reference(second))
        self.assertEqual(moved['pane_id'], replacement['pane_id'])
        self.assertEqual(moved['terminal_id'], replacement['terminal_id'])
        self.assertNotEqual(reference(moved), reference(replacement))
        stopped = trace['claude-stopped']['result']['pane']
        self.assertNotIn('agent_session', stopped)

    def test_codex_launch_modes_and_native_resume_move(self):
        trace = read('session-identity-trace.json')
        self.assertIsNone(reference(agent(trace, 'codex-final')),
                          'Observed shared-daemon launch must not be labelled supported')
        local = agent(trace, 'codex-local-final')
        probe = agent(trace, 'hook-agent')
        resumed = agent(trace, 'hook-resumed')
        moved = agent(trace, 'codex-after-move')
        self.assertTrue(reference(local))
        self.assertTrue(reference(probe))
        self.assertEqual(local['cwd'], probe['cwd'])
        self.assertNotEqual(reference(local), reference(probe))
        self.assertEqual(reference(probe), reference(resumed))
        self.assertEqual(reference(resumed), reference(moved))
        self.assertEqual(resumed['terminal_id'], moved['terminal_id'])
        self.assertNotEqual(resumed['pane_id'], moved['pane_id'])
        hooks = read('codex-session-hook.jsonl')
        self.assertEqual(hooks[0]['source'], 'startup')
        self.assertEqual(hooks[-1]['source'], 'resume')
        self.assertEqual(hooks[0]['session_id'], hooks[-1]['session_id'])
        self.assertEqual(hooks[-1]['environment']['CODEX_THREAD_ID'], '')

    def test_claude_tool_pairing_question_and_interrupt(self):
        rows = read('claude-transcript.jsonl')
        blocks = [b for r in rows for b in r.get('message', {}).get('content', []) if isinstance(b, dict)]
        calls = {b['id']: b for b in blocks if b.get('type') == 'tool_use'}
        results = [b for b in blocks if b.get('type') == 'tool_result']
        self.assertIn('Read', {c['name'] for c in calls.values()})
        self.assertIn('AskUserQuestion', {c['name'] for c in calls.values()})
        for result in results:
            self.assertIn(result['tool_use_id'], calls)
        self.assertIn('Read', {calls[r['tool_use_id']]['name'] for r in results})
        self.assertTrue(any('[Request interrupted by user]' in json.dumps(r) for r in rows))
        state = agent(read('session-identity-trace.json'), 'claude-question-state')
        self.assertEqual(state['agent_status'], 'blocked')

    def test_codex_rollout_and_paginated_history_shapes(self):
        rows = read('codex-rollout.jsonl')
        calls = {r['payload']['call_id'] for r in rows if r['type'] == 'response_item' and r['payload']['type'] == 'custom_tool_call'}
        results = {r['payload']['call_id'] for r in rows if r['type'] == 'response_item' and r['payload']['type'] == 'custom_tool_call_output'}
        self.assertTrue(calls)
        self.assertTrue(results.issubset(calls))
        self.assertTrue(any(r.get('payload', {}).get('type') == 'task_complete' for r in rows))
        items = read('codex-paginated-items.json')
        self.assertTrue({'userMessage', 'agentMessage', 'commandExecution'}.issubset({i['type'] for i in items}))
        self.assertEqual(len(items), len({i['id'] for i in items}))
        self.assertIn('item_json', {c['name'] for c in read('codex-history-schema.json')['thread_items']})

    def test_subscription_snapshot_and_cancellation_evidence(self):
        frames = read('herdr-events.json')
        self.assertEqual(frames[0]['result']['type'], 'subscription_started')
        self.assertEqual(frames[1]['event'], 'workspace_renamed')
        self.assertNotIn('seq', frames[1])
        self.assertNotIn('sequence', frames[1])
        self.assertEqual(frames[2]['result']['type'], 'session_snapshot')
        self.assertEqual(frames[3]['result']['type'], 'session_snapshot')

    def test_ssh_probe_is_transport_evidence_only(self):
        result = read('ssh-probe.json')
        self.assertEqual(result['greeting']['text'], '猫 ✓')
        self.assertEqual(result['echo']['id'], 'probe')
        self.assertEqual(result['exit_code'], 0)
        self.assertNotEqual(result['cancellation_exit_code'], 0)
        self.assertEqual(result['reconnect_exit_code'], 0)
        self.assertEqual(result['reconnect_output'], 'alive')
        self.assertIn('Citadel', result['client'])
        self.assertIn('not live-tested', result['client'])

    def test_simulated_framing_rotation_and_unknown_cases(self):
        case = read('boundary-cases.json')
        self.assertIn('simulated', case['origin'])
        frame = bytes.fromhex(case['framing']['record_utf8_hex'])
        split, before_newline = case['framing']['split_offsets']
        with self.assertRaises(UnicodeDecodeError):
            frame[:split].decode('utf-8')
        self.assertNotIn(b'\n', frame[:before_newline])
        self.assertEqual(json.loads(frame.decode('utf-8'))['fixture_unicode'], '猫 ✓')
        self.assertTrue(case['filesystem']['rename_replace_changes_inode'])
        self.assertTrue(case['filesystem']['truncate_preserves_inode'])
        self.assertEqual(case['filesystem']['size_after'], 0)
        self.assertEqual(case['unknown_record']['type'], 'future_record')

    def test_sanitizer_preserves_identity_links_and_empty_context(self):
        from sanitize import sanitize
        original = {'session_id': 'secret-session', 'agent_session': {'value': 'secret-session'},
                    'environment': {'CODEX_THREAD_ID': '', 'HERDR_SOCKET_PATH': '/Users/example/socket'},
                    'message': {'content': [{'type': 'text', 'text': 'private conversation'}]},
                    'input': '{"cmd":"cat /Users/example/private.txt"}',
                    'base_instructions': 'private instructions'}
        result = sanitize(original)
        self.assertEqual(result['session_id'], result['agent_session']['value'])
        self.assertEqual(result['environment']['CODEX_THREAD_ID'], '')
        self.assertNotIn('private conversation', json.dumps(result))
        self.assertNotIn('/Users/', json.dumps(result))
        self.assertNotIn('base_instructions', result)
        self.assertEqual(json.loads(result['input'])['cmd'], 'Fixture text')

    def test_captured_schema_has_sources_but_no_approval_method(self):
        schema = read('herdr-schema.json')
        self.assertEqual(schema['protocol'], 22)
        def consts(node):
            if isinstance(node, dict):
                if 'const' in node: yield node['const']
                for value in node.values(): yield from consts(value)
            elif isinstance(node, list):
                for value in node: yield from consts(value)
        methods = set(consts(schema['schemas']['request']))
        self.assertTrue({'events.subscribe', 'session.snapshot', 'pane.report_agent_session'}.issubset(methods))
        self.assertFalse(any('approval' in m or 'question' in m for m in methods))

if __name__ == '__main__':
    unittest.main(verbosity=2)
