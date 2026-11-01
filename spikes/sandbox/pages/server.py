#!/usr/bin/env python3
"""Serves the test pages inside the sandbox and appends their events to a log."""
import http.server
import json
import os
import pathlib

SESSION = pathlib.Path(os.environ["KEEK_SESSION"])
PAGES = pathlib.Path(__file__).parent
EVENTS = SESSION / "page-events.jsonl"
STATE = SESSION / "page-state"


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, body, content_type="text/plain"):
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/state":
            self.reply(STATE.read_bytes() if STATE.exists() else b"white")
            return
        page = PAGES / self.path.lstrip("/")
        if page.suffix == ".html" and page.exists():
            self.reply(page.read_bytes(), "text/html")
        else:
            self.send_error(404)

    def do_POST(self):
        body = self.rfile.read(int(self.headers["Content-Length"]))
        with EVENTS.open("a") as log:
            log.write(json.dumps(json.loads(body)) + "\n")
        self.reply(b"ok")


http.server.ThreadingHTTPServer(("127.0.0.1", 8765), Handler).serve_forever()
