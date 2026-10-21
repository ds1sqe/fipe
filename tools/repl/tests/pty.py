"""Drive the line editor through a terminal: python3 tools/repl/tests/pty.py BIN...

Types a line, edits it with Left and Backspace, runs it, recalls it with
Up and runs it again, enters a two-line input, clears a line with Ctrl-C
and ends with Ctrl-D; checks the output and that the terminal settings
come back as they were.
"""
import os
import select
import subprocess
import sys
import termios

LEFT, UP = b"\x1b[D", b"\x1b[A"


def session(binary):
    master, slave = os.openpty()
    before = termios.tcgetattr(slave)
    child = subprocess.Popen(
        [binary], stdin=slave, stdout=slave, stderr=slave, start_new_session=True
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
    send(b"\x04")
    assert child.wait(timeout=10) == 0, binary
    assert termios.tcgetattr(slave) == before, f"{binary}: terminal not restored"
    os.close(master)
    os.close(slave)
    print(f"{binary}: line editor session passed")


for binary in sys.argv[1:]:
    session(binary)
