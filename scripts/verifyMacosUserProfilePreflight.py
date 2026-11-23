#!/usr/bin/env python3
"""Fail closed if the disposable macOS user's Fylura fixture paths are occupied."""

import os
import sys


def fail(message: str) -> None:
    print(message, file=sys.stderr)
    raise SystemExit(1)


if len(sys.argv) < 3:
    fail("Expected a disposable home and at least one Fylura test path.")

home = os.path.abspath(sys.argv[1])
current = home
while True:
    if os.path.islink(current):
        fail("Refusing a symlink in the disposable runner home: " + current)
    parent = os.path.dirname(current)
    if parent == current:
        break
    current = parent

for candidate in sys.argv[2:]:
    path = os.path.abspath(candidate)
    if os.path.commonpath((home, path)) != home:
        fail("Refusing a Fylura test path outside the disposable runner home: " + path)
    current = path
    while current != home:
        if os.path.islink(current):
            fail("Refusing a symlink in a Fylura lifecycle test path: " + current)
        parent = os.path.dirname(current)
        if parent == current:
            fail("Could not find the disposable home in a Fylura test path: " + path)
        current = parent
    if os.path.lexists(path):
        fail("Refusing a pre-existing Fylura lifecycle test path: " + path)
