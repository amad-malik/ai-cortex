#!/usr/bin/env python3
"""Run stdin Python with every urllib opener mapped to local fake responses."""
import io
import os
from pathlib import Path
import sys
import urllib.request

fixture = Path(os.environ["FAKE_STATE_DIR"]) / "fetch-fixtures"
directory = Path(fixture.read_text().strip())

class FakeOpener:
    def __init__(self, handlers):
        self.redirect = next((handler for handler in handlers if isinstance(handler, urllib.request.HTTPRedirectHandler)),
                             urllib.request.HTTPRedirectHandler())

    def open(self, request, timeout=None):
        url = request.full_url if hasattr(request, "full_url") else request
        if os.environ.get("FAKE_HTTP_REDIRECT") == "1":
            self.redirect.redirect_request(urllib.request.Request(url), None, 302, "Found", {}, "http://evil.example/metadata")
        if url.startswith("https://mirrors.fedoraproject.org/metalink?"):
            name = "metalink"
        elif url.startswith("https://mirror.example/epel/"):
            suffix = url.removeprefix("https://mirror.example/epel/")
            name = {"repodata/repomd.xml": "repomd", "repodata/repomd.xml.asc": "signature",
                    "repodata/primary.xml.gz": "primary", "Packages/p/podman-compose.rpm": "package"}.get(suffix)
        elif url.startswith("https://dl.rockylinux.org/pub/rocky/10.2/"):
            suffix = url.removeprefix("https://dl.rockylinux.org/pub/rocky/10.2/")
            repo, rest = suffix.split("/x86_64/os/", 1)
            channel = {"BaseOS": "baseos", "AppStream": "appstream", "CRB": "crb", "extras": "extras"}.get(repo)
            item = {"repodata/repomd.xml": "repomd", "repodata/repomd.xml.asc": "signature",
                    "repodata/primary.xml.gz": "primary"}.get(rest)
            name = f"{channel}-{item}" if channel and item else None
        else:
            name = None
        if name is None:
            raise OSError("unexpected fake URL")
        return io.BytesIO((directory / name).read_bytes())

urllib.request.build_opener = lambda *args: FakeOpener(args)
sys.argv[0] = "-"
exec(compile(sys.stdin.read(), "<fake-stdin>", "exec"), {"__name__": "__main__"})
