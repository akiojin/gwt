#!/usr/bin/env python3
"""Bound one nextest --list invocation and diagnose stalls before killing it."""

import os
import select
import shlex
import signal
import subprocess
import sys
import time


def forward_before_deadline(output, fd, expires_at):
    os.set_blocking(fd, False)
    remaining = memoryview(output)
    while remaining:
        wait = expires_at - time.monotonic()
        if wait <= 0 or not select.select([], [fd], [], wait)[1]:
            return False
        try:
            remaining = remaining[os.write(fd, remaining):]
        except BlockingIOError:
            continue
    return True


def main(deadline, command):
    # Keep the test process's pipes separate from nextest's, so an unresponsive
    # child cannot hold nextest's output open after this wrapper exits.
    process = subprocess.Popen(
        command, start_new_session=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE
    )
    expires_at = time.monotonic() + deadline
    stderr_fd = sys.stderr.fileno()
    try:
        stdout, stderr = process.communicate(timeout=deadline)
    except subprocess.TimeoutExpired as error:
        diagnostic = (
            f"Test enumeration timed out after {deadline:g}s (pid {process.pid}): "
            f"{shlex.join(command)}\n"
        ).encode()
        try:
            columns = (
                "pid,ppid,stat,wchan:32,args"
                if sys.platform == "linux"
                else "pid,ppid,stat,wchan,args"
            )
            state = subprocess.run(
                ["ps", "-o", columns, "-p", str(process.pid)],
                capture_output=True,
                timeout=5,
                check=False,
            )
            diagnostic += state.stdout + state.stderr
        except (OSError, subprocess.TimeoutExpired) as snapshot_error:
            diagnostic += f"Unable to capture process state: {snapshot_error}\n".encode()
        finally:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            diagnostic += f"Process {process.pid} did not exit after SIGKILL\n".encode()
        if error.stderr:
            diagnostic += b"Tail of test stderr:\n" + error.stderr[-2048:]
        forward_before_deadline(diagnostic, stderr_fd, time.monotonic() + 1)
        return 124

    if forward_before_deadline(stdout, sys.stdout.fileno(), expires_at) and forward_before_deadline(
        stderr, stderr_fd, expires_at
    ):
        return process.returncode if process.returncode >= 0 else 128 - process.returncode

    # The child has exited, but nextest may be blocked reading our output.
    # Report without waiting on another pipe if it also stopped reading stderr.
    diagnostic = (
        f"Wrapper pid {os.getpid()} timed out forwarding test enumeration output "
        f"after {deadline:g}s: {shlex.join(command)}\n"
    ).encode()
    forward_before_deadline(diagnostic, stderr_fd, time.monotonic() + 1)
    return 124


if __name__ == "__main__":
    sys.exit(main(float(sys.argv[1]), sys.argv[2:]))
