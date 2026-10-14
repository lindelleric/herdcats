#!/usr/bin/env python3
"""Probe an explicitly selected disposable Herdr session (never the default).

Example: python3 probe_herdr.py --session hc-phase0-test --workspace w1 --output /tmp/probe.json
The selected workspace's label changes; use only a test workspace you created.
"""
import argparse
import json
from pathlib import Path
import socket
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--session', required=True)
    parser.add_argument('--workspace', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if not args.session.startswith('hc-phase0-') or '/' in args.session:
        parser.error('session must be an explicitly created hc-phase0-* test session')
    listing = json.loads(subprocess.check_output(['herdr', 'session', 'list', '--json']))
    selected = next((s for s in listing['sessions'] if s['name'] == args.session and s['running']), None)
    if not selected:
        parser.error('named test session is not running')

    def connect():
        client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        client.settimeout(5)
        try:
            client.connect(selected['socket_path'])
        except BaseException:
            client.close()
            raise
        return client

    def send(client, request_id, method, params):
        client.sendall((json.dumps({'id': request_id, 'method': method, 'params': params}) + '\n').encode())

    def rpc(request_id, method):
        with connect() as client, client.makefile('rb') as stream:
            send(client, request_id, method, {})
            return json.loads(stream.readline())

    with connect() as client, client.makefile('rb') as stream:
        send(client, 'subscribe', 'events.subscribe', {'subscriptions': [{'type': 'workspace.renamed'}]})
        frames = [json.loads(stream.readline())]
        if frames[0].get('result', {}).get('type') != 'subscription_started':
            raise RuntimeError('subscription rejected')
        subprocess.run(['herdr', '--session', args.session, 'workspace', 'rename', args.workspace,
                        'phase0-event-' + str(time.time_ns())], check=True, capture_output=True)
        frames.append(json.loads(stream.readline()))
        frames.append(rpc('snapshot', 'session.snapshot'))
    frames.append(rpc('after-close', 'session.snapshot'))
    args.output.write_text(json.dumps(frames, indent=2) + '\n')
    print('Captured subscription, event, snapshot and post-cancellation snapshot.')

if __name__ == '__main__':
    main()
