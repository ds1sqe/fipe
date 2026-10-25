#!/usr/bin/env python3
"""Installed Linux DAP smoke: --binary ABSOLUTE_BINARY --root EXTERNAL_ASSETS.

Reuse the recorder's framing/request flow with deadlines and owned-child cleanup.
No Cargo, checkout discovery, provider loading or installed-package fallback.
"""
import argparse
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import tempfile
import time


class Adapter:
    def __init__(self, command, cwd, seconds=30):
        self.command = command
        self.cwd = cwd
        self.child = None
        self.deadline = time.monotonic() + seconds
        self.buffer = bytearray()
        self.errors = bytearray()
        self.stderr_open = True
        self.seq = 0
        self.pending = []
        self.messages = []

    def __enter__(self):
        # Defer termination only until detached-child cleanup owns the handle.
        pending = []
        previous = signal.signal(signal.SIGTERM, lambda signum, frame: pending.append(signum))
        try:
            self.child = subprocess.Popen(
                self.command, cwd=self.cwd, stdin=subprocess.PIPE,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        except BaseException:
            signal.signal(signal.SIGTERM, previous)
            raise
        try:
            signal.signal(signal.SIGTERM, previous)
            if pending:
                signal.raise_signal(signal.SIGTERM)
            return self
        except BaseException:
            self.__exit__(*sys.exc_info())
            raise

    def __exit__(self, *error):
        try:
            os.killpg(self.child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            self.child.wait(timeout=2)
        finally:
            for stream in (self.child.stdin, self.child.stdout, self.child.stderr):
                stream.close()

    def read_more(self):
        streams = [self.child.stdout]
        if self.stderr_open:
            streams.append(self.child.stderr)
        remaining = self.deadline - time.monotonic()
        assert remaining > 0, "adapter deadline exceeded"
        ready, _, _ = select.select(streams, [], [], remaining)
        assert ready, "adapter deadline exceeded"
        for stream in ready:
            data = os.read(stream.fileno(), 65536)
            if stream is self.child.stderr:
                self.errors.extend(data)
                self.stderr_open = bool(data)
            elif data:
                self.buffer.extend(data)
            else:
                return False
        return True

    def receive(self):
        while b"\r\n\r\n" not in self.buffer:
            assert len(self.buffer) <= 8192, "invalid adapter header"
            if not self.read_more():
                assert not self.buffer, "truncated adapter header"
                return None
        header, _, _ = self.buffer.partition(b"\r\n\r\n")
        assert len(header) <= 8192, "invalid adapter header"
        lengths = [int(value) for line in header.split(b"\r\n")
                   for name, separator, value in [line.partition(b":")]
                   if separator and name.lower() == b"content-length"]
        assert len(lengths) == 1 and 0 <= lengths[0] <= 64 * 1024 * 1024, header
        size = len(header) + 4 + lengths[0]
        while len(self.buffer) < size:
            assert self.read_more(), "truncated adapter body"
        body = bytes(self.buffer[len(header) + 4:size])
        del self.buffer[:size]
        message = json.loads(body)
        assert isinstance(message, dict), message
        self.messages.append(message)
        print("<- " + json.dumps(message), flush=True)
        return message

    def request(self, command, arguments=None, success=True):
        self.seq += 1
        message = {"seq": self.seq, "type": "request", "command": command}
        if arguments is not None:
            message["arguments"] = arguments
        body = json.dumps(message).encode()
        print("-> " + json.dumps(message), flush=True)
        self.child.stdin.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
        self.child.stdin.flush()
        while True:
            reply = self.receive()
            assert reply is not None, "adapter closed before response"
            if reply.get("type") == "response" and reply.get("request_seq") == self.seq:
                assert reply["command"] == command, reply
                assert reply["success"] is success, reply
                return reply
            self.pending.append(reply)

    def event(self, name):
        while True:
            for index, message in enumerate(self.pending):
                if message.get("type") == "event" and message.get("event") == name:
                    return self.pending.pop(index).get("body", {})
            message = self.receive()
            assert message is not None, "adapter closed before " + name
            self.pending.append(message)

    def output(self, category):
        return [m["body"]["output"] for m in self.messages
                if m.get("event") == "output" and m["body"]["category"] == category]

    def finish(self):
        self.request("disconnect")
        self.child.stdin.close()
        while self.receive() is not None:
            pass
        remaining = self.deadline - time.monotonic()
        assert remaining > 0, "adapter deadline exceeded"
        assert self.child.wait(timeout=remaining) == 0
        # A descendant retaining stderr must not turn EOF accounting into a hang.
        while self.stderr_open:
            remaining = self.deadline - time.monotonic()
            assert remaining > 0, "adapter stderr deadline exceeded"
            ready, _, _ = select.select([self.child.stderr], [], [], remaining)
            assert ready, "adapter stderr deadline exceeded"
            data = os.read(self.child.stderr.fileno(), 65536)
            self.errors.extend(data)
            self.stderr_open = bool(data)
        assert not self.errors, bytes(self.errors)


def launch(adapter, arguments):
    adapter.request("initialize", {"adapterID": "fipe"})
    adapter.request("launch", arguments)
    adapter.event("initialized")


def replay_arguments(root):
    return {"program": str(root / "Documentation/examples/02-outcomes.fipe"),
            "host": str(root / "tests/fixtures/host"),
            "trace": str(root / "tests/fixtures/traces/02-outcomes.txt")}


def replay(binary, root):
    with Adapter([str(binary)], root) as adapter:
        launch(adapter, replay_arguments(root))
        adapter.request("configurationDone")
        adapter.request("threads")
        adapter.event("terminated")
        adapter.event("exited")
        adapter.finish()
        assert_replay_output(adapter, root)
    print("PASS replay", flush=True)


def assert_replay_output(adapter, root):
    lines = (root / "tests/fixtures/traces/02-outcomes.txt").read_text().splitlines()
    expected = [line + "\n" for line in lines if line.startswith("call ")]
    assert adapter.output("console") == expected + [lines[0] + "\n"]
    for category, label in [("stdout", "success "), ("stderr", "error ")]:
        literal = next(line[len(label):] for line in lines if line.startswith(label))
        assert "".join(adapter.output(category)) == json.loads(literal)


def pipe_at(adapter, position):
    top = adapter.request("stackTrace", {"threadId": 1})["body"]["stackFrames"][0]
    assert (top["line"], top["column"]) == position, top
    scopes = adapter.request("scopes", {"frameId": top["id"]})["body"]["scopes"]
    reference = next(s["variablesReference"] for s in scopes if s["name"] == "Pipe")
    variables = adapter.request("variables", {"variablesReference": reference})
    return [(v["name"], v["value"]) for v in variables["body"]["variables"]]


def stepping(binary, root):
    with Adapter([str(binary)], root) as adapter:
        arguments = replay_arguments(root)
        launch(adapter, arguments)
        reply = adapter.request("setBreakpoints", {
            "source": {"path": arguments["program"]},
            "breakpoints": [{"line": 7, "column": 20}]})
        assert reply["body"]["breakpoints"][0]["verified"] is True
        adapter.request("configurationDone")
        assert adapter.event("stopped")["reason"] == "breakpoint"
        entry = 'Entry "config.toml" "./config.toml" 12 60'
        assert pipe_at(adapter, (7, 20)) == [
            ("mark", "|:"), ("in", "Just (" + entry + ")"),
            ("side", "good"), ("stage", "runs"), ("out", "not yet")]
        adapter.request("next", {"threadId": 1})
        assert adapter.event("stopped")["reason"] == "step"
        assert pipe_at(adapter, (7, 27)) == [
            ("mark", "|!"), ("in", 'Just "port = 8080\\n"'),
            ("side", "good"), ("stage", "skipped"), ("out", "not yet")]
        adapter.request("next", {"threadId": 1})
        assert adapter.event("stopped")["reason"] == "step"
        assert pipe_at(adapter, (7, 46))[0] == ("mark", "||")
        adapter.request("continue", {"threadId": 1})
        adapter.event("terminated")
        adapter.finish()
        assert_replay_output(adapter, root)
    print("PASS stepping", flush=True)


def standard_live(binary, root):
    with tempfile.TemporaryDirectory(prefix="installed-dap-", dir=root) as scratch:
        parent = Path(scratch)
        for case in ("healthy", "denied", "input", "sentinel"):
            directory = parent / case
            directory.mkdir()
            program = directory / "live.fipe"
            sources = {
                "healthy": 'use cli\n"first"\n"written\\n" | put ./live.txt\n'
                           'cli.args | join " "\ns => Err (s | text.trim)\n',
                "denied": '"x" | put ../escape.txt\n',
                "input": 'n => n + 1\n',
                "sentinel": '"unexpected" | put ./refused-effect.txt\nn => n + 1\n',
            }
            program.write_text(sources[case])
            arguments = {"program": str(program), "cwd": str(directory)}
            if case == "healthy":
                arguments.update(args=["a", "b"], input=" in \n")
            elif case in ("input", "sentinel"):
                arguments["input"] = "x"
            with Adapter([str(binary)], directory) as adapter:
                adapter.request("initialize", {"adapterID": "fipe"})
                if case in ("input", "sentinel"):
                    reply = adapter.request("launch", arguments, success=False)
                else:
                    adapter.request("launch", arguments)
                    adapter.event("initialized")
                    adapter.request("configurationDone")
                    if case == "healthy":
                        adapter.event("terminated")
                        exited = adapter.event("exited")
                    else:
                        stopped = adapter.event("stopped")
                adapter.finish()
                if case in ("input", "sentinel"):
                    assert "host input enters only a String or Vec<U8> spot" in reply["message"]
                    assert not (directory / "refused-effect.txt").exists()
                    assert not adapter.output("stdout") and not adapter.output("stderr")
                    assert not adapter.output("console")
                elif case == "healthy":
                    assert exited["exitCode"] == 1
                    assert "".join(adapter.output("stdout")) == "first\na b\n"
                    assert "".join(adapter.output("stderr")) == "in\n"
                    assert (directory / "live.txt").read_text() == "written\n"
                    console = adapter.output("console")
                    assert console[0].startswith("call commands.read")
                    assert console[-1] == 'value Err "in"\n'
                else:
                    assert stopped["reason"] == "exception"
                    assert stopped["description"].endswith(
                        "host: ../escape.txt is outside the write roots")
                    assert not (parent / "escape.txt").exists()
            print("PASS standard-live/" + case, flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--root", required=True, type=Path)
    args = parser.parse_args()
    assert args.binary.is_absolute() and args.root.is_absolute()
    assert args.binary.is_file() and args.root.is_dir()
    replay(args.binary, args.root)
    stepping(args.binary, args.root)
    standard_live(args.binary, args.root)
    print("PASS installed DAP: 3 cases, 6 processes", flush=True)


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, lambda signum, frame: sys.exit(128 + signum))
    main()
