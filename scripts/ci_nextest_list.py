#!/usr/bin/env python3
"""Bound one nextest --list invocation and diagnose stalls before killing it."""

import os
import shlex
import signal
import subprocess
import sys


def main(deadline, command):
    # Keep the test process's pipes separate from nextest's, so an unresponsive
    # child cannot hold nextest's output open after this wrapper exits.
    process = subprocess.Popen(
        command, start_new_session=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE
    )
    try:
        stdout, stderr = process.communicate(timeout=deadline)
    except subprocess.TimeoutExpired as error:
        print(
            f"Test enumeration timed out after {deadline:g}s (pid {process.pid}): "
            f"{shlex.join(command)}",
            file=sys.stderr,
        )
        sys.stderr.buffer.write(error.stderr or b"")
        sys.stderr.flush()
        try:
            columns = (
                "pid,ppid,stat,wchan:32,args"
                if sys.platform == "linux"
                else "pid,ppid,stat,wchan,args"
            )
            subprocess.run(
                ["ps", "-o", columns, "-p", str(process.pid)],
                stdout=sys.stderr,
                timeout=5,
                check=False,
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            print(f"Unable to capture process state: {error}", file=sys.stderr)
        finally:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            print(f"Process {process.pid} did not exit after SIGKILL", file=sys.stderr)
        return 124

    sys.stdout.buffer.write(stdout)
    sys.stderr.buffer.write(stderr)
    return process.returncode if process.returncode >= 0 else 128 - process.returncode


if __name__ == "__main__":
    sys.exit(main(float(sys.argv[1]), sys.argv[2:]))
