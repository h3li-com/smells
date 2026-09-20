#!/usr/bin/env python3
"""Verify that PyPI files exactly match locally signed release wheels."""

from __future__ import annotations

import hashlib
import json
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path


def fail(message: str) -> None:
    raise SystemExit(f"PyPI verification failed: {message}")


if len(sys.argv) != 3:
    fail("usage: verify-pypi-release.py VERSION WHEEL_DIRECTORY")

version = sys.argv[1]
directory = Path(sys.argv[2])
local = {
    wheel.name: hashlib.sha256(wheel.read_bytes()).hexdigest()
    for wheel in directory.glob("*.whl")
}
if len(local) != 5:
    fail(f"expected five local wheels, found {len(local)}")

metadata: dict[str, object] | None = None
url = f"https://pypi.org/pypi/smells/{version}/json"
for _attempt in range(10):
    try:
        with urllib.request.urlopen(url, timeout=15) as response:  # noqa: S310
            metadata = json.load(response)
        break
    except urllib.error.HTTPError as error:
        if error.code != 404:
            raise
        time.sleep(3)

if metadata is None:
    fail(f"smells {version} did not become visible")

urls = metadata.get("urls")
if not isinstance(urls, list):
    fail("release metadata has no file list")

remote: dict[str, str] = {}
for item in urls:
    if not isinstance(item, dict) or item.get("packagetype") != "bdist_wheel":
        fail("release contains a non-wheel distribution")
    filename = item.get("filename")
    digests = item.get("digests")
    if not isinstance(filename, str) or not isinstance(digests, dict):
        fail("release file metadata is incomplete")
    digest = digests.get("sha256")
    if not isinstance(digest, str):
        fail(f"release file has no SHA-256 digest: {filename}")
    remote[filename] = digest

if remote != local:
    fail(f"wheel digest set differs: local={sorted(local)} remote={sorted(remote)}")

print(f"verified five PyPI wheels for smells {version}")
