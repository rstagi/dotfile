#!/bin/sh
# Use the shared JSONL recipe for a new rollout copy.
if [ "${1:-}" = --rewrite ]; then
  exec "$(dirname "$0")/relocate-claude.sh" "$@"
fi
exec python3 - "$0" "$@" <<'PY'
from collections import Counter
from datetime import date
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import uuid


def main():
    if len(sys.argv) != 2:
        raise SystemExit('Use no arguments, or --rewrite SRC DST OLD_HOME NEW_HOME.')
    executable = shutil.which('codex')
    if not executable:
        raise SystemExit('The codex executable is missing.')
    os.umask(0o077)
    root = Path(tempfile.mkdtemp(prefix='s1-codex-')).resolve()
    print(f'Sandbox: {root}', flush=True)
    live_config = Path(os.environ.get('CODEX_HOME', str(Path.home() / '.codex')))
    homes = [root / 'home-a', root / 'home-b']
    for home in homes:
        (home / '.codex').mkdir(parents=True)
        credential = live_config / 'auth.json'
        if credential.is_file():
            shutil.copyfile(credential, home / '.codex' / 'auth.json')
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(('CLAUDE_', 'CODEX_', 'WS_', 'LOOP_', 'OTEL_', 'GROK_'))
           and key not in ('CLAUDECODE', 'BASH_ENV', 'ENV', 'GIT_DIR', 'GIT_WORK_TREE')}
    repo_a, repo_b = [home / 'repo' for home in homes]
    repo_a.mkdir()
    subprocess.run(['git', 'init', '-q', str(repo_a)], check=True, env=env)
    (repo_a / 'marker.txt').write_text('This file contains synthetic S1 data.\n')
    word = 'TOPAZ-' + uuid.uuid4().hex[:12].upper()
    flags = ['--ignore-user-config', '--ignore-rules', '--json', '-m', 'gpt-6.1-sol',
             '-c', 'model_reasoning_effort="low"', '-c', 'approval_policy="never"',
             '-c', 'sandbox_mode="read-only"']

    def call(home, repo, args, label):
        call_env = dict(env, HOME=str(home), CODEX_HOME=str(home / '.codex'),
                        CLAUDE_CONFIG_DIR=str(home / '.claude'),
                        XDG_CONFIG_HOME=str(home / '.config'))
        with (root / (label + '.jsonl')).open('w') as output, \
                (root / (label + '.stderr')).open('w') as errors:
            result = subprocess.run([executable, 'exec', *flags, '-o',
                                     str(root / (label + '.txt')), *args],
                                    cwd=repo, env=call_env, stdin=subprocess.DEVNULL,
                                    stdout=output, stderr=errors, timeout=180)
        if result.returncode:
            raise SystemExit(f'Agent call failed. Inspect {root / (label + ".stderr")} and {label}.jsonl.')
        events = [json.loads(line) for line in (root / (label + '.jsonl')).read_text().splitlines()]
        if any(event.get('type') in ('error', 'turn.failed') for event in events):
            raise SystemExit(f'Agent returned an error. Inspect {root / (label + ".jsonl")}.')
        threads = [event['thread_id'] for event in events if event.get('type') == 'thread.started']
        if len(threads) != 1:
            raise SystemExit('Expected exactly one thread identity in the agent output.')
        return threads[0], (root / (label + '.txt')).read_text().strip()

    session, start_answer = call(homes[0], repo_a,
        [f'Remember the code word {word}. Read {repo_a}/marker.txt with the shell tool once. '
         'Reply only STORED. Do not write files or read other files.'], 'start')
    candidates = list((homes[0] / '.codex' / 'sessions').rglob('*' + session + '*.jsonl'))
    if len(candidates) != 1:
        raise SystemExit('Expected exactly one source rollout.')
    source = candidates[0]
    source_bytes = source.read_bytes()
    (root / 'source.jsonl').write_bytes(source_bytes)
    shutil.copytree(repo_a, repo_b)
    relative = source.relative_to(homes[0] / '.codex')
    target = homes[1] / '.codex' / relative
    target.parent.mkdir(parents=True)
    subprocess.run([str(Path(sys.argv[1]).resolve()), '--rewrite', str(source), str(target),
                    str(homes[0]), str(homes[1])], check=True, stdin=subprocess.DEVNULL)
    relocated_bytes = target.read_bytes()
    (root / 'relocated-before-resume.jsonl').write_bytes(relocated_bytes)
    target_files = sorted(str(path.relative_to(homes[1] / '.codex'))
                          for path in (homes[1] / '.codex').rglob('*') if path.is_file())
    resumed_session, answer = call(homes[1], repo_b,
        ['resume', session, 'Read marker.txt in your current working directory with the shell tool once. '
         'Do not read other files. What code word did I ask you to remember? '
         'Reply with the code word only. Do not write files.'], 'resume')
    rows = [json.loads(line) for line in source_bytes.decode().splitlines()]
    summary = dict(date=str(date.today()), agent_home_mode='sandbox', session_id=session,
                   code_word=word, requested_model='gpt-6.1-sol',
                   version=subprocess.check_output([executable, '--version'], text=True,
                                                   stdin=subprocess.DEVNULL).strip(),
                   authentication='Copied auth.json when present. Inherited OPENAI_API_KEY when present.',
                   source_rollout=str(source), target_rollout=str(target),
                   target_files_before_resume=target_files,
                   source_record_count=len(rows),
                   record_types=dict(Counter(row['type'] for row in rows)),
                   source_sha256=hashlib.sha256(source_bytes).hexdigest(),
                   relocated_before_resume_sha256=hashlib.sha256(relocated_bytes).hexdigest(),
                   source_unchanged=source.read_bytes() == source_bytes,
                   old_home_occurrences_after_rewrite=relocated_bytes.count(str(homes[0]).encode()),
                   marker_contains_code_word=word in (repo_b / 'marker.txt').read_text(),
                   start_result=start_answer, resume_result=answer,
                   same_session=resumed_session == session, recall_pass=answer == word,
                   agent_calls=2)
    # Commit only this allowlisted summary. Raw rollouts can contain private data.
    redacted = json.loads(json.dumps(summary).replace(str(root), '<sandbox>'))
    (root / 'summary.json').write_text(json.dumps(redacted, indent=2) + '\n')
    print(json.dumps(redacted, indent=2))
    if (not summary['recall_pass'] or not summary['same_session']
            or not summary['source_unchanged'] or summary['marker_contains_code_word']
            or summary['old_home_occurrences_after_rewrite'] or start_answer != 'STORED'):
        raise SystemExit('Relocation failed a recall, identity, source, or path check.')


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        raise SystemExit(f'Relocation failed: {error}')
PY
