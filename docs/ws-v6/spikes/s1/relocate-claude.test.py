"""Check the transcript relocation interface without agent calls."""
import json
import os
import stat
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('relocate-claude.sh')


class RelocationTest(unittest.TestCase):
    def test_rewrite_preserves_session_and_message_chain(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / 'source.jsonl'
            target = Path(root) / 'target.jsonl'
            rows = [
                {'type': 'user', 'sessionId': 'session-1', 'uuid': 'message-1',
                 'parentUuid': None, 'cwd': '/tmp/home-a/repo',
                 'message': {'content': 'Remember SAPPHIRE-731.'}},
                {'type': 'assistant', 'uuid': 'message-2', 'parentUuid': 'message-1',
                 'message': {'content': [{'type': 'tool_use', 'input': {
                     'file_path': '/tmp/home-a/repo/marker.txt'}}]},
                 'text': 'Working directory: /tmp/home-a/repo\n',
                 'unrelated': '/tmp/home-archive/keep'},
            ]
            original = ''.join(json.dumps(row) + '\n' for row in rows)
            source.write_text(original)
            result = subprocess.run([str(SCRIPT), '--rewrite', str(source), str(target),
                                     '/tmp/home-a', '/tmp/home-b'], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(stat.S_IMODE(target.stat().st_mode), 0o600)
            actual = read_rows(target)
            self.assertEqual(actual[0]['cwd'], '/tmp/home-b/repo')
            self.assertEqual(actual[1]['message']['content'][0]['input']['file_path'],
                             '/tmp/home-b/repo/marker.txt')
            self.assertEqual(actual[1]['text'], 'Working directory: /tmp/home-b/repo\n')
            self.assertEqual(actual[1]['unrelated'], '/tmp/home-archive/keep')
            self.assertEqual(actual[0]['sessionId'], 'session-1')
            self.assertEqual(actual[1]['parentUuid'], 'message-1')
            self.assertEqual(actual[0]['message']['content'], 'Remember SAPPHIRE-731.')
            self.assertEqual(source.read_text(), original)

    def test_rewrite_respects_path_boundaries(self):
        cases = {
            '/tmp/home-a': '/tmp/home-b',
            '/tmp/home-a/repo': '/tmp/home-b/repo',
            'cd "/tmp/home-a/repo"': 'cd "/tmp/home-b/repo"',
            "open '/tmp/home-a/repo'": "open '/tmp/home-b/repo'",
            'PWD=/tmp/home-a/repo': 'PWD=/tmp/home-b/repo',
            'path:/tmp/home-a/repo': 'path:/tmp/home-b/repo',
            '<cwd>/tmp/home-a/repo</cwd>': '<cwd>/tmp/home-b/repo</cwd>',
            'file:///tmp/home-a/repo/marker.txt': 'file:///tmp/home-b/repo/marker.txt',
            '/Volumes/backup/tmp/home-a/repo': '/Volumes/backup/tmp/home-a/repo',
            'x/tmp/home-a/repo': 'x/tmp/home-a/repo',
            '/tmp/home-a backup/repo': '/tmp/home-a backup/repo',
            "/tmp/home-a's backup/repo": "/tmp/home-a's backup/repo",
            '/tmp/home-a-archive/keep': '/tmp/home-a-archive/keep',
            '/tmp/home-a.old/keep': '/tmp/home-a.old/keep',
        }
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / 'source.jsonl'
            target = Path(root) / 'target.jsonl'
            source.write_text(json.dumps({'values': list(cases)}) + '\n')
            result = run_rewrite(source, target)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(dict(zip(cases, read_rows(target)[0]['values'])), cases)

    def test_rewrite_rejects_homes_that_need_escaping(self):
        for old_home, new_home in [('/tmp/home-a', '/tmp/home"b'),
                                   ('/tmp/home-a', '/tmp/home\\b'),
                                   ('/tmp/home-a', '/tmp/home\tb'),
                                   ('/tmp/home"a', '/tmp/home-b')]:
            with self.subTest(old_home=old_home, new_home=new_home), \
                    tempfile.TemporaryDirectory() as root:
                source = Path(root) / 'source.jsonl'
                target = Path(root) / 'target.jsonl'
                source.write_text(json.dumps({'arguments': json.dumps(
                    {'cmd': 'cat /tmp/home-a/repo/marker.txt'})}) + '\n')
                result = run_rewrite(source, target, old_home, new_home)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('Home paths must not contain', result.stderr)
                self.assertFalse(target.exists())

    def test_rewrite_keeps_unicode_line_separators_inside_records(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / 'source.jsonl'
            target = Path(root) / 'target.jsonl'
            rows = [{'text': 'a\u2028b\u2029c\x85d /tmp/home-a/repo'}, {'n': 2}]
            source.write_bytes(''.join(json.dumps(row, ensure_ascii=False) + '\n'
                                       for row in rows).encode())
            result = run_rewrite(source, target)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(read_rows(target),
                             [{'text': 'a\u2028b\u2029c\x85d /tmp/home-b/repo'}, {'n': 2}])

    def test_rewrite_keeps_lone_surrogates_representable(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / 'source.jsonl'
            target = Path(root) / 'target.jsonl'
            source.write_text('{"text":"\\ud83d /tmp/home-a/repo"}\n{"n":"\u00e9"}\n')
            result = run_rewrite(source, target)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(read_rows(target),
                             [{'text': '\ud83d /tmp/home-b/repo'}, {'n': '\u00e9'}])

    def test_rewrite_refuses_existing_target(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / 'source.jsonl'
            target = Path(root) / 'existing.jsonl'
            source.write_text('{"cwd":"/tmp/home-a/repo"}\n')
            target.write_text('Preserve this existing transcript.\n')
            result = subprocess.run([str(SCRIPT), '--rewrite', str(source), str(target),
                                     '/tmp/home-a', '/tmp/home-b'], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(target.read_text(), 'Preserve this existing transcript.\n')
            self.assertEqual(source.read_text(), '{"cwd":"/tmp/home-a/repo"}\n')

    def test_experiment_does_not_send_script_source_to_agent(self):
        with tempfile.TemporaryDirectory() as root:
            fake = Path(root) / 'claude'
            fake.write_text('''#!/usr/bin/env python3
import json, os, pathlib, re, sys
args = sys.argv[1:]
if args == ['--version']:
    print('synthetic-test-version')
    sys.exit(0)
try:
    inherited = os.pread(0, 100000, 0)
except OSError:
    inherited = b''
if inherited or sys.stdin.read():
    print('The agent received unexpected standard input.', file=sys.stderr)
    sys.exit(1)
config = pathlib.Path(os.environ['CLAUDE_CONFIG_DIR'])
project = re.sub(r'[^a-zA-Z0-9]', '-', os.getcwd())
if '--session-id' in args:
    session = args[args.index('--session-id') + 1]
    word = re.search(r'code word (SAPPHIRE-[A-F0-9]+)', args[-1]).group(1)
    folder = config / 'projects' / project
    folder.mkdir(parents=True)
    (folder / (session + '.jsonl')).write_text(json.dumps({
        'sessionId': session, 'cwd': os.getcwd(), 'word': word}) + '\\n')
    answer = 'STORED'
else:
    session = args[args.index('--resume') + 1]
    row = json.loads((config / 'projects' / project / (session + '.jsonl')).read_text())
    assert row['cwd'] == os.getcwd()
    answer = row['word']
print(json.dumps({'session_id': session, 'result': answer, 'is_error': False}))
''')
            fake.chmod(0o755)
            env = dict(os.environ, PATH=root + os.pathsep + os.environ['PATH'], TMPDIR=root)
            result = subprocess.run([str(SCRIPT)], env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn('"recall_pass": true', result.stdout)


def run_rewrite(source, target, old_home='/tmp/home-a', new_home='/tmp/home-b'):
    return subprocess.run([str(SCRIPT), '--rewrite', str(source), str(target), old_home, new_home],
                          capture_output=True, text=True)


def read_rows(path):
    # Split on LF only. JSON strings can hold raw U+0085, U+2028 and U+2029.
    return [json.loads(line) for line in path.read_bytes().decode().split('\n') if line]


if __name__ == '__main__':
    unittest.main()
