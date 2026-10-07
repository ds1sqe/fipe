"""Check platform reads: python3 tools/repl/check-platform.py SCRATCH BIN..."""
import os
from pathlib import Path
import subprocess
import sys

scratch = Path(sys.argv[1])
directory = scratch / "directory.fipe"
directory.mkdir(exist_ok=True)
for binary in sys.argv[2:]:
    fd = os.open(directory, os.O_RDONLY)
    try:
        result = subprocess.run([binary], stdin=fd, capture_output=True, timeout=10)
    finally:
        os.close(fd)
    assert result.returncode == 2, result
    assert b"cannot read standard input" in result.stdout, result
    result = subprocess.run(
        [binary], input=f":load {directory}\n".encode(), capture_output=True, timeout=10
    )
    assert result.returncode == 0 and b"cannot read file" in result.stdout, result
    result = subprocess.run(
        [binary], input=b":host tests/fixtures/host\0suffix\n", capture_output=True, timeout=10
    )
    assert result.returncode == 0 and b"NUL" in result.stdout, result
    assert b"host: " not in result.stdout, result
    result = subprocess.run(
        [binary, "--host", "missing\n:q"], input=b":t 1\n", capture_output=True, timeout=10
    )
    assert result.returncode == 0 and b"1: Integer" in result.stdout, result
    print(f"{binary}: 4 platform checks passed")
