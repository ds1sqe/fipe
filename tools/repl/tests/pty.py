"""Drive the line editor through a terminal: python3 tools/repl/tests/pty.py BIN...

Run from the repository root. With NO_COLOR set: types a line, edits it
with Left and Backspace, runs it, recalls it with Up and runs it again,
enters a two-line input, clears a line with Ctrl-C and ends with Ctrl-D;
checks the output and that the terminal settings come back as they were.
With colour and the fixture host: completes `fs.li` to `fs.list` with
Tab and checks that a line's echo is coloured by token class.
"""
import os
import select
import subprocess
import sys
import termios

LEFT, UP = b"\x1b[D", b"\x1b[A"


def session(binary, args=(), color=False):
    """Start `binary` on a new terminal; return until, send and end."""
    master, slave = os.openpty()
    before = termios.tcgetattr(slave)
    env = {k: v for k, v in os.environ.items() if k != "NO_COLOR"}
    if not color:
        env["NO_COLOR"] = "1"
    child = subprocess.Popen(
        [binary, *args],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        start_new_session=True,
        env=env,
    )
    seen = b""

    def until(text):
        nonlocal seen
        while text not in seen:
            ready, _, _ = select.select([master], [], [], 10)
            assert ready, f"{binary}: waited for {text!r}, saw {seen!r}"
            seen += os.read(master, 4096)
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
        os.close(master)
        os.close(slave)

    return until, send, end


def editing(binary):
    until, send, end = session(binary)
    until(b"fipe> ")
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


def completing(binary):
    until, send, end = session(binary, ["--host", "tests/fixtures/host"], True)
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


for binary in sys.argv[1:]:
    editing(binary)
    completing(binary)
