#!/usr/bin/env python3
"""Create redacted shape fixtures from explicitly supplied disposable captures.

Usage: python3 sanitize.py --input raw.jsonl --output sanitized.jsonl
Never point this utility at arbitrary user conversations for fixture generation.
"""
import argparse
import json
from pathlib import Path
import uuid

NAMESPACE = uuid.UUID('8244c068-2069-4d83-aecc-96b7e7fe754f')
IDENTITIES = {'id', 'uuid', 'parentUuid', 'sessionId', 'session_id', 'thread_id',
              'threadId', 'turn_id', 'turnId', 'root_turn_id', 'parent_id',
              'call_id', 'callId', 'tool_use_id', 'requestId', 'toolUseID',
              'item_id', 'itemId', 'messageId', 'leafUuid', 'promptId',
              'processId', 'response_id', 'clientId', 'value', 'terminal_id'}
ENUMS = {'type', 'subtype', 'role', 'status', 'phase', 'kind', 'agent',
         'agent_status', 'source', 'hook_event_name', 'permissionMode', 'mode',
         'version', 'cli_version', 'history_mode', 'event'}
OMIT = {'base_instructions', 'creator_user_id', 'creator_account_id',
        'runtime_workspace_roots', 'permission_profile', 'sandbox_policy',
        'collaboration_mode', 'model', 'requestedModel', 'advisorModel', 'git',
        'hookAdditionalContext', 'hookInfos', 'hookErrors', 'wireToolInputs',
        'rate_limits', 'memoryCitation'}

def sanitize(value, key=''):
    if isinstance(value, dict):
        return {k: sanitize(v, k) for k, v in value.items() if k not in OMIT}
    if isinstance(value, list):
        return [sanitize(v, key) for v in value]
    if not isinstance(value, str) or not value:
        return value
    if value.startswith('[Request interrupted'):
        return '[Request interrupted by user]'
    if key in IDENTITIES:
        return str(uuid.uuid5(NAMESPACE, value))
    if key in ENUMS and len(value) < 80 and all(c.isalnum() or c in '_.:-' for c in value):
        return value
    if key in {'pane_id', 'workspace_id', 'tab_id'}:
        return value
    if key == 'name' and value in {'Read', 'AskUserQuestion', 'exec_command', 'functions.exec_command'}:
        return value
    if key.lower().endswith(('path', 'cwd', 'dir')) or key in {'cwd', 'foreground_cwd', 'scriptPath'}:
        return '/fixture/work'
    if key == 'timestamp' or key.endswith('At'):
        return '2026-10-13T00:00:00Z'
    if key in {'input', 'arguments', 'output'}:
        try:
            return json.dumps(sanitize(json.loads(value)), ensure_ascii=False)
        except (ValueError, TypeError):
            pass
    return 'Fixture text'

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.input.resolve() == args.output.resolve():
        parser.error('input and output must be different files')
    text = args.input.read_text()
    if args.input.suffix == '.jsonl':
        result = ''.join(json.dumps(sanitize(json.loads(line)), ensure_ascii=False) + '\n'
                         for line in text.splitlines())
    else:
        result = json.dumps(sanitize(json.loads(text)), ensure_ascii=False, indent=2) + '\n'
    args.output.write_text(result)

if __name__ == '__main__':
    main()
