#!/usr/bin/env python3
"""Drive the fipe-dap binary over stdio through a whole debugging
session of 02-outcomes.fipe and print every message, one JSON object
per line, prefixed by its direction ("->" sent, "<-" received).

Usage: scripts/record.py <path to fipe-dap> <repository root>
"""
import json
import subprocess
import sys

adapter, root = sys.argv[1], sys.argv[2]
child = subprocess.Popen([adapter], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
seq = 0
pending = []


def send(command, arguments=None):
    global seq
    seq += 1
    message = {"seq": seq, "type": "request", "command": command}
    if arguments is not None:
        message["arguments"] = arguments
    body = json.dumps(message).encode()
    child.stdin.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    child.stdin.flush()
    print("-> " + json.dumps(message), flush=True)
    return seq


def receive():
    length = None
    while True:
        line = child.stdout.readline()
        if not line:
            return None
        line = line.strip()
        if not line:
            break
        name, _, value = line.decode().partition(":")
        if name.lower() == "content-length":
            length = int(value)
    message = json.loads(child.stdout.read(length))
    print("<- " + json.dumps(message), flush=True)
    return message


def request(command, arguments=None):
    number = send(command, arguments)
    while True:
        message = receive()
        if message is None:
            sys.exit("the adapter closed its output")
        pending.append(message)
        if message.get("type") == "response" and message["request_seq"] == number:
            return message


def wait(event):
    while True:
        for message in pending:
            if message.get("type") == "event" and message["event"] == event:
                pending.remove(message)
                return message
        message = receive()
        if message is None:
            sys.exit("no %s event" % event)
        pending.append(message)


program = root + "/Documentation/examples/02-outcomes.fipe"
request("initialize", {"adapterID": "fipe"})
request("launch", {
    "program": program,
    "host": root + "/tests/fixtures/host",
    "trace": root + "/tests/fixtures/traces/02-outcomes.txt",
})
wait("initialized")
locations = request("breakpointLocations", {"source": {"path": program}, "line": 7})
column = locations["body"]["breakpoints"][0]["column"]
request("setBreakpoints", {"source": {"path": program},
                           "breakpoints": [{"line": 7, "column": column}]})
request("configurationDone")
wait("stopped")
for step in range(3):
    frames = request("stackTrace", {"threadId": 1})["body"]["stackFrames"]
    scopes = request("scopes", {"frameId": frames[0]["id"]})["body"]["scopes"]
    for scope in scopes:
        if scope["name"] in ("Pipe", "Effects", "Budget"):
            request("variables", {"variablesReference": scope["variablesReference"]})
    if step < 2:
        request("next", {"threadId": 1})
        wait("stopped")
request("evaluate", {"expression": "default_config", "frameId": frames[0]["id"]})
request("continue", {"threadId": 1})
wait("terminated")
request("disconnect")
child.stdin.close()
child.wait()
sys.exit(child.returncode)
