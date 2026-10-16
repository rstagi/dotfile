#!/usr/bin/env python3
"""Save OTLP/HTTP request bodies without decoding them. Use only on loopback."""

import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import time
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--port", type=int, default=4318)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    server = ThreadingHTTPServer(("127.0.0.1", args.port), CaptureHandler)
    server.output = args.output
    print(json.dumps({"host": "127.0.0.1", "port": server.server_port}), flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


class CaptureHandler(BaseHTTPRequestHandler):
    def do_POST(self):
        body = self.read_body()
        content_type = self.headers.get("Content-Type", "application/octet-stream")
        extension = ".json" if "json" in content_type else ".pb"
        stem = f"{time.time_ns()}-{uuid.uuid4().hex}"
        body_file = stem + extension
        (self.server.output / body_file).write_bytes(body)
        metadata = {
            "path": self.path,
            "content_type": content_type,
            "content_encoding": self.headers.get("Content-Encoding"),
            "received_unix_ns": time.time_ns(),
            "body_file": body_file,
            "body_bytes": len(body),
            "sha256": hashlib.sha256(body).hexdigest(),
        }
        (self.server.output / (stem + ".meta.json")).write_text(json.dumps(metadata, indent=2) + "\n")
        response = b"{}" if "json" in content_type else b""
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(response)))
        self.end_headers()
        self.wfile.write(response)

    def read_body(self):
        if self.headers.get("Transfer-Encoding", "").lower() != "chunked":
            return self.rfile.read(int(self.headers.get("Content-Length", "0")))
        chunks = []
        while True:
            size = int(self.rfile.readline().split(b";", 1)[0], 16)
            if size == 0:
                while self.rfile.readline() not in (b"\r\n", b"\n", b""):
                    pass
                return b"".join(chunks)
            chunk = self.rfile.read(size)
            if len(chunk) != size or self.rfile.read(2) != b"\r\n":
                raise ValueError("The request body is incomplete.")
            chunks.append(chunk)

    def log_message(self, format, *args):
        pass


if __name__ == "__main__":
    main()
