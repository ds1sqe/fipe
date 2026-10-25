"""Drive the line editor through a terminal: python3 tools/repl/tests/pty.py BIN...

Run from the repository root. With NO_COLOR set: types a line, edits it
with Left and Backspace, runs it, recalls it with Up and runs it again,
enters a two-line input, clears a line with Ctrl-C and ends with Ctrl-D;
checks the output and that the terminal settings come back as they were.
With colour and the fixture host: completes `fs.li` to `fs.list` with
Tab and checks that a line's echo is coloured by token class.
"""
import os
import signal
from contextlib import contextmanager
import select
import subprocess
import sys
import termios
import tempfile
import time
from pathlib import Path

LEFT, UP = b"\x1b[D", b"\x1b[A"


@contextmanager
def session(binary, args=(), color=False):
    """Start `binary` on a new terminal; return until, send and end."""
    # Defer termination only until detached-child cleanup owns the handle.
    pending = []
    previous = signal.signal(signal.SIGTERM, lambda signum, frame: pending.append(signum))
    master = slave = None
    try:
        master, slave = os.openpty()
        before = termios.tcgetattr(slave)
    except BaseException:
        for descriptor in (master, slave):
            if descriptor is not None:
                os.close(descriptor)
        signal.signal(signal.SIGTERM, previous)
        raise
    env = {k: v for k, v in os.environ.items() if k != "NO_COLOR"}
    if not color:
        env["NO_COLOR"] = "1"
    try:
        child = subprocess.Popen(
            [binary, *args],
            stdin=slave,
            stdout=slave,
            stderr=slave,
            start_new_session=True,
            env=env,
        )
    except BaseException:
        os.close(master)
        os.close(slave)
        signal.signal(signal.SIGTERM, previous)
        raise
    seen = b""

    def until(text):
        nonlocal seen
        deadline = time.monotonic() + 10
        while text not in seen:
            remaining = deadline - time.monotonic()
            assert remaining > 0, f"{binary}: waited for {text!r}, saw {seen!r}"
            ready, _, _ = select.select([master], [], [], remaining)
            assert ready, f"{binary}: waited for {text!r}, saw {seen!r}"
            data = os.read(master, 4096)
            assert data, f"{binary}: closed before {text!r}, saw {seen!r}"
            seen += data
        at = seen.index(text) + len(text)
        shown, seen = seen[:at], seen[at:]
        return shown

    def send(*keys):
        for key in keys:
            os.write(master, key)

    def end():
        send(b"\x04")
        assert child.wait(timeout=10) == 0, binary
        assert termios.tcgetattr(slave) == before, f"{binary}: terminal not restored"

    try:
        signal.signal(signal.SIGTERM, previous)
        if pending:
            signal.raise_signal(signal.SIGTERM)
        yield until, send, end
    finally:
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            child.wait(timeout=2)
        finally:
            os.close(master)
            os.close(slave)


def editing(binary, args=()):
    with session(binary, args) as (until, send, end):
        assert b":help lists the commands" in until(b"fipe> ")
        # "12 * 3", four Lefts to after "12", Backspace and "4": "14 * 3".
        send(b"12 * 3", *[LEFT] * 4, b"\x7f", b"4", b"\r")
        assert b"fipe> 14 * 3\x1b[K" in until(b"....> ")
        send(b"\r")
        until(b"42\r\nfipe> ")
        send(UP)
        until(b"\rfipe> 14 * 3\x1b[K\r\x1b[12C")
        send(b"\r\r")
        until(b"42\r\nfipe> ")
        send(b"(1 +\r")
        until(b"....> ")
        send(b"2)\r")
        until(b"....> ")
        send(b"\r")
        until(b"3\r\nfipe> ")
        send(b"abc", b"\x03")
        until(b"^C\r\nfipe> ")
        end()
    print(f"{binary}: line editor session passed")


def completing(binary, args=("--host", "tests/fixtures/host")):
    with session(binary, args, True) as (until, send, end):
        until(b"fipe> ")
        send(b"fs.li\t")
        until(b"\rfipe> fs.list\x1b[K\r\x1b[13C")
        # Ctrl-U clears the line; the echo after Enter is coloured.
        send(b"\x15", b'"a" ++ "b"\r')
        green, blue, off = b"\x1b[32m", b"\x1b[34m", b"\x1b[0m"
        echo = until(b"....> ")
        coloured = (
            b"fipe> " + green + b'"a"' + off + b" " + blue + b"++" + off
            + b" " + green + b'"b"' + off + b"\x1b[K"
        )
        assert coloured in echo, echo
        send(b"\r")
        until(b'"ab"\r\nfipe> ')
        end()
    print(f"{binary}: completion and colour session passed")




def output_failure_worker(command, env=None):
    """Observe raw mode before injected run_driver failure restores the live PTY."""
    master, slave = os.openpty()
    before = termios.tcgetattr(slave)
    child = subprocess.Popen(command, stdin=slave, stdout=slave, stderr=slave,
                             start_new_session=True, env=env)
    try:
        seen = b""
        while b"RAW-READY" not in seen:
            assert select.select([master], [], [], 10)[0], seen
            seen += os.read(master, 4096)
        raw = termios.tcgetattr(slave)
        assert not raw[3] & termios.ICANON and not raw[3] & termios.ECHO
        assert raw != before
        os.write(master, b"x")
        assert child.wait(timeout=10) == 2
        assert termios.tcgetattr(slave) == before
    finally:
        if child.poll() is None:
            child.kill()
            child.wait()
        os.close(master)
        os.close(slave)


def pipe_failures(binary, cli=False):
    """Observed write/read errors are fatal; failed output prevents later effects."""
    with tempfile.TemporaryDirectory(prefix="fipe-repl-output-") as directory:
        command = [binary, "repl", "--allow-write", directory] if cli else [binary]
        text = b'put out "before"\n\nput ./sentinel "no"\n\n' if cli else b'1\n\n2\n\n'
        with open("/dev/full", "wb") as sink:
            failed = subprocess.run(command, input=text, stdout=sink, stderr=subprocess.PIPE,
                                    cwd=directory, timeout=10)
        assert failed.returncode == 2, failed
        assert not Path(directory, "sentinel").exists()
        healthy = subprocess.run(command, input=text, capture_output=True, cwd=directory, timeout=10)
        assert healthy.returncode == 0, healthy
        if cli:
            assert Path(directory, "sentinel").read_text() == "no"
        else:
            assert b"1\n" in healthy.stdout and b"2\n" in healthy.stdout
        # An observed read error: stdin is a directory, not an EOF-producing pipe.
        fd = os.open(directory, os.O_RDONLY)
        try:
            bad_read = subprocess.run(command, stdin=fd, capture_output=True, cwd=directory, timeout=10)
            assert bad_read.returncode == 2, bad_read
            assert b"cannot read standard input" in bad_read.stdout
        finally:
            os.close(fd)
        for restore_signals in [True, False]:
            Path(directory, "sentinel").unlink(missing_ok=True)
            read_fd, write_fd = os.pipe()
            os.close(read_fd)
            try:
                broken = subprocess.run(command, input=text, stdout=write_fd, stderr=subprocess.PIPE,
                                        cwd=directory, timeout=10, restore_signals=restore_signals)
                if restore_signals:
                    assert broken.returncode in [2, -13], broken
                else:
                    assert broken.returncode == 2, broken
                assert not Path(directory, "sentinel").exists()
                print(f"{binary}: closed pipe restore_signals={restore_signals}: {broken.returncode}")
            finally:
                os.close(write_fd)
    print(f"{binary}: pipe failure controls passed")


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, lambda signum, frame: sys.exit(128 + signum))
    args = iter(sys.argv[1:])
    for binary in args:
        if binary == "--worker-std":
            env = dict(os.environ, FIPE_OUTPUT_WORKER="1")
            output_failure_worker([next(args), "--exact", "output_tests::output_pty_worker", "--ignored", "--nocapture"], env)
        elif binary == "--worker-linux":
            output_failure_worker([next(args)])
        elif binary == "--cli-interactive":
            command = next(args)
            editing(command, ["repl"])
            completing(command, ["repl"])
        elif binary == "--cli":
            pipe_failures(next(args), True)
        else:
            editing(binary)
            completing(binary)
            pipe_failures(binary)
