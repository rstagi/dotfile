#!/bin/sh
# Run the S1 experiment, or relocate a transcript with --rewrite SRC DST OLD_HOME NEW_HOME.
exec python3 - "$@" <<'PY'
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import uuid


def main():
    if len(sys.argv) == 6 and sys.argv[1] == '--rewrite':
        rewrite(*sys.argv[2:])
    elif len(sys.argv) == 1:
        experiment()
    else:
        raise SystemExit('Use no arguments, or --rewrite SRC DST OLD_HOME NEW_HOME.')


def experiment():
    executable = shutil.which('claude')
    if not executable:
        raise SystemExit('The claude executable is missing.')
    os.umask(0o077)
    root = Path(tempfile.mkdtemp(prefix='s1-claude-')).resolve()
    print(f'Sandbox: {root}', flush=True)
    live_config = Path(os.environ.get('CLAUDE_CONFIG_DIR', str(Path.home() / '.claude')))
    homes = [root / 'home-a', root / 'home-b']
    for home in homes:
        (home / '.claude').mkdir(parents=True)
        credential = live_config / '.credentials.json'
        if credential.is_file():
            shutil.copy2(credential, home / '.claude' / '.credentials.json')
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(('CLAUDE_', 'CODEX_', 'WS_', 'LOOP_', 'OTEL_', 'GROK_'))
           and key not in ('CLAUDECODE', 'BASH_ENV', 'ENV', 'GIT_DIR', 'GIT_WORK_TREE')}
    env.update(DISABLE_TELEMETRY='1', CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC='1')
    repo_a, repo_b = [home / 'repo' for home in homes]
    repo_a.mkdir()
    subprocess.run(['git', 'init', '-q', str(repo_a)], check=True, env=env)
    (repo_a / 'marker.txt').write_text('This file contains synthetic S1 data.\n')
    session = str(uuid.uuid4())
    word = 'SAPPHIRE-' + uuid.uuid4().hex[:12].upper()
    flags = ['--safe-mode', '--setting-sources', '', '--strict-mcp-config',
             '--mcp-config', '{"mcpServers":{}}', '--tools', 'Read',
             '--permission-mode', 'dontAsk', '--no-chrome', '--disable-slash-commands',
             '--model', 'haiku', '--effort', 'low', '--output-format', 'json']

    def call(home, repo, args, label):
        call_env = dict(env, HOME=str(home), CLAUDE_CONFIG_DIR=str(home / '.claude'),
                        CODEX_HOME=str(home / '.codex'), XDG_CONFIG_HOME=str(home / '.config'))
        with (root / (label + '.json')).open('w') as output, \
                (root / (label + '.stderr')).open('w') as errors:
            result = subprocess.run([executable, *flags, *args], cwd=repo, env=call_env,
                                    stdin=subprocess.DEVNULL, stdout=output, stderr=errors,
                                    timeout=180)
        if result.returncode:
            raise SystemExit(f'Agent call failed. Inspect {root / (label + ".stderr")} and {label}.json.')
        data = json.loads((root / (label + '.json')).read_text())
        if data.get('is_error'):
            raise SystemExit(f'Agent returned an error. Inspect {root / (label + ".json")}.')
        return data

    start = call(homes[0], repo_a, ['--session-id', session, '-p',
                 f'Remember the code word {word}. Read {repo_a}/marker.txt once. '
                 'Reply only STORED. Do not write files.'], 'start')
    candidates = list((homes[0] / '.claude' / 'projects').rglob(session + '.jsonl'))
    if len(candidates) != 1:
        raise SystemExit('Expected exactly one source transcript.')
    source = candidates[0]
    shutil.copy2(source, root / 'source.jsonl')
    shutil.copytree(repo_a, repo_b)
    # Short ASCII sandbox paths avoid Claude's >200-character project-name hashing.
    project_b = re.sub(r'[^a-zA-Z0-9]', '-', str(repo_b))
    if len(project_b) > 200:
        raise SystemExit('The sandbox project name exceeds 200 characters.')
    target_dir = homes[1] / '.claude' / 'projects' / project_b
    target_dir.mkdir(parents=True)
    target = target_dir / (session + '.jsonl')
    rewrite(source, target, str(homes[0]), str(homes[1]))
    shutil.copy2(target, root / 'relocated-before-resume.jsonl')
    # The target config has no source transcript. The source home stays intact.
    resumed = call(homes[1], repo_b, ['--resume', session, '-p',
                   'What code word did I ask you to remember? Reply with the code word only.'], 'resume')
    answer = resumed.get('result', '').strip()
    summary = dict(agent_home_mode='sandbox', session_id=session, code_word=word,
                   version=subprocess.check_output([executable, '--version'], text=True).strip(),
                   source_project=source.parent.name, target_project=project_b,
                   source_transcript=str(source), target_transcript=str(target),
                   start_result=start.get('result'), resume_result=answer,
                   same_session=resumed.get('session_id') == session,
                   recall_pass=answer == word, agent_calls=2)
    (root / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps(summary, indent=2))
    if not summary['recall_pass'] or not summary['same_session']:
        raise SystemExit('Relocation failed the recall or session identity check.')


def rewrite(source, target, old_home, new_home):
    old_home = os.path.normpath(old_home)
    new_home = os.path.normpath(new_home)
    if not old_home.startswith('/') or not new_home.startswith('/'):
        raise ValueError('Home paths must be absolute.')
    if not old_home.strip('/') or not new_home.strip('/') or old_home == new_home:
        raise ValueError('Home paths must differ and must not be the root directory.')
    # Raw quotes, backslashes or control characters break serialized JSON inside strings.
    if any(char in '"\\' or ord(char) < 0x20 or 0x7f <= ord(char) <= 0x9f
           for char in old_home + new_home):
        raise ValueError('Home paths must not contain quotes, backslashes or control characters.')
    # Match whole paths only. A path character before the home makes it part of another path.
    # A file:// URL is the one allowed path prefix. A home must end with / or the string.
    pattern = re.compile(r'(?:(?<=file://)|(?<![\w.~/+@%-]))' + re.escape(old_home)
                         + r'(?=/|\Z)')

    def replace(value):
        if isinstance(value, str):
            return pattern.sub(lambda match: new_home, value)
        if isinstance(value, list):
            return [replace(item) for item in value]
        if isinstance(value, dict):
            return {key: replace(item) for key, item in value.items()}
        return value

    # Split on LF only. JSON strings can hold raw U+0085, U+2028 and U+2029.
    lines = Path(source).read_bytes().decode().split('\n')
    output = b''.join(encode(replace(json.loads(line))) for line in lines if line)
    # Create the target only after every record is ready. A failure leaves no partial file.
    descriptor = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, 'wb') as stream:
        stream.write(output)


def encode(row):
    try:
        return (json.dumps(row, ensure_ascii=False) + '\n').encode()
    except UnicodeEncodeError:
        # Escape lone surrogates. UTF-8 cannot encode them raw.
        return (json.dumps(row) + '\n').encode()


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        raise SystemExit(f'Relocation failed: {error}')
PY
