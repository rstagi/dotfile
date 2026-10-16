"""Check rollout relocation through the script interface without model calls."""
import json
import os
import stat
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('relocate-codex.sh')


class RelocationTest(unittest.TestCase):
    def test_rewrite_preserves_thread_and_tool_links(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / 'source.jsonl'
            target = Path(root) / 'target.jsonl'
            rows = [
                {'type': 'session_meta', 'payload': {
                    'id': 'thread-1', 'cwd': '/tmp/home-a/repo'}},
                {'type': 'turn_context', 'payload': {'cwd': '/tmp/home-a/repo'}},
                {'type': 'response_item', 'payload': {
                    'type': 'function_call', 'call_id': 'call-1',
                    'arguments': json.dumps({'cmd': 'cat /tmp/home-a/repo/marker.txt'})}},
                {'type': 'response_item', 'payload': {
                    'type': 'function_call_output', 'call_id': 'call-1',
                    'output': 'S1 synthetic marker'}},
                {'type': 'response_item', 'payload': {
                    'type': 'reasoning', 'encrypted_content': 'opaque-content'}},
                {'type': 'response_item', 'payload': {
                    'type': 'message', 'content': [
                        {'text': 'Remember TOPAZ-731. Cwd: /tmp/home-a/repo\n'}]},
                 'unrelated': '/tmp/home-archive/keep'},
            ]
            original = ''.join(json.dumps(row) + '\n' for row in rows)
            source.write_text(original)
            result = subprocess.run([str(SCRIPT), '--rewrite', str(source), str(target),
                                     '/tmp/home-a', '/tmp/home-b'], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            # Split on LF only. JSON strings can hold raw U+0085, U+2028 and U+2029.
            actual = [json.loads(line) for line in target.read_bytes().decode().split('\n') if line]
            self.assertEqual(actual[0]['payload'], {'id': 'thread-1', 'cwd': '/tmp/home-b/repo'})
            self.assertEqual(actual[1]['payload']['cwd'], '/tmp/home-b/repo')
            self.assertEqual(json.loads(actual[2]['payload']['arguments']),
                             {'cmd': 'cat /tmp/home-b/repo/marker.txt'})
            self.assertEqual(actual[2]['payload']['call_id'], 'call-1')
            self.assertEqual(actual[3], rows[3])
            self.assertEqual(actual[4], rows[4])
            self.assertEqual(actual[5]['payload']['content'][0]['text'],
                             'Remember TOPAZ-731. Cwd: /tmp/home-b/repo\n')
            self.assertEqual(actual[5]['unrelated'], '/tmp/home-archive/keep')
            self.assertEqual(stat.S_IMODE(target.stat().st_mode), 0o600)
            self.assertEqual(source.read_text(), original)

    def test_experiment_resumes_from_rollout_without_source_index(self):
        with tempfile.TemporaryDirectory() as root:
            fake = Path(root) / 'codex'
            fake.write_text('''#!/usr/bin/env python3
import json, os, pathlib, re, sys
args = sys.argv[1:]
if args == ['--version']:
    print('synthetic-test-version')
    sys.exit(0)
assert not sys.stdin.read(), 'The agent received unexpected standard input.'
assert 'CODEX_THREAD_ID' not in os.environ
assert 'WS_SESSION_ID' not in os.environ
home = pathlib.Path(os.environ['CODEX_HOME'])
output = pathlib.Path(args[args.index('-o') + 1])
session = '11111111-1111-4111-8111-111111111111'
folder = home / 'sessions' / '2026' / '10' / '16'
rollout = folder / ('rollout-test-' + session + '.jsonl')
if 'resume' not in args:
    word = re.search(r'code word (TOPAZ-[A-F0-9]+)', args[-1]).group(1)
    folder.mkdir(parents=True)
    # Raw line separators are valid inside JSON strings. Readers must split on LF only.
    rollout.write_bytes((json.dumps({'type': 'session_meta', 'payload': {
        'id': session, 'cwd': os.getcwd(), 'word': word, 'note': 'a\\u2028b\\x85c'}},
        ensure_ascii=False) + '\\n').encode())
    # A source index exists. A resume must work without its transfer.
    (home / 'state_5.sqlite').write_text('source-only index')
    answer = 'STORED'
else:
    assert args[args.index('resume') + 1] == session
    assert not list(home.glob('*.sqlite'))
    assert sorted(p.name for p in home.iterdir()) in (['sessions'], ['auth.json', 'sessions'])
    row = json.loads(rollout.read_bytes().decode())
    assert row['payload']['cwd'] == os.getcwd()
    answer = row['payload']['word']
    with rollout.open('a') as stream:
        stream.write(json.dumps({'type': 'turn_context', 'payload': {'cwd': os.getcwd()}}) + '\\n')
output.write_text(answer + '\\n')
sys.stdout.buffer.write((json.dumps({'type': 'thread.started', 'thread_id': session,
                                     'note': 'a\\u2029b'}, ensure_ascii=False) + '\\n').encode())
sys.stdout.flush()
print(json.dumps({'type': 'turn.completed', 'usage': {'input_tokens': 1}}))
''')
            fake.chmod(0o755)
            env = dict(os.environ, PATH=root + os.pathsep + os.environ['PATH'], TMPDIR=root,
                       CODEX_THREAD_ID='parent-thread', WS_SESSION_ID='parent-session')
            result = subprocess.run([str(SCRIPT)], env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn('"recall_pass": true', result.stdout)
            self.assertIn('"source_unchanged": true', result.stdout)
            self.assertIn('"same_session": true', result.stdout)
            self.assertIn('"old_home_occurrences_after_rewrite": 0', result.stdout)


if __name__ == '__main__':
    unittest.main()
