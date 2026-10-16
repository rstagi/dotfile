"""Check rollout relocation through the script interface without model calls."""
import json
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
            actual = [json.loads(line) for line in target.read_text().splitlines()]
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


if __name__ == '__main__':
    unittest.main()
