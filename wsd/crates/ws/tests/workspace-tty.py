"""Run a workspace command with a controlling terminal in the test sandbox."""

import errno
import os
import pty
import select
import signal
import sys
import time

binary, prompt, answer, *arguments = sys.argv[1:]
pid, terminal = pty.fork()
if pid == 0:
    os.execv(binary, [binary, *arguments])

output = bytearray()
sent = False
deadline = time.monotonic() + 10
try:
    while True:
        if time.monotonic() >= deadline:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
            raise TimeoutError("The workspace command did not complete.")
        if select.select([terminal], [], [], 0.1)[0]:
            try:
                chunk = os.read(terminal, 65536)
            except OSError as error:
                if error.errno != errno.EIO:
                    raise
                chunk = b""
            if not chunk:
                _, status = os.waitpid(pid, 0)
                sys.stdout.buffer.write(output)
                sys.exit(os.waitstatus_to_exitcode(status))
            output.extend(chunk)
        if not sent and prompt.encode() in output:
            os.write(terminal, answer.encode() + b"\n")
            sent = True
finally:
    os.close(terminal)
