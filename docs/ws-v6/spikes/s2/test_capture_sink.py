"""Test the sink through HTTP and its output directory."""

import gzip
import http.client
import json
from pathlib import Path
import select
import subprocess
import sys
import tempfile
import unittest

SINK = Path(__file__).with_name("capture-sink.py")


class CaptureSinkTest(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.output = Path(self.directory.name)
        self.process = subprocess.Popen(
            [sys.executable, "-I", str(SINK), "--output", str(self.output), "--port", "0"],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )
        self.addCleanup(self.stop)
        ready, _, _ = select.select([self.process.stdout], [], [], 5)
        self.assertTrue(ready, "The sink did not report its address.")
        line = self.process.stdout.readline()
        self.assertTrue(line, self.process.stderr.read() if self.process.poll() is not None else "No address.")
        self.address = json.loads(line)

    def stop(self):
        self.process.terminate()
        self.process.communicate(timeout=5)

    def post(self, path, body, content_type, **kwargs):
        connection = http.client.HTTPConnection("127.0.0.1", self.address["port"], timeout=5)
        self.addCleanup(connection.close)
        connection.request("POST", path, body, {"Content-Type": content_type}, **kwargs)
        response = connection.getresponse()
        self.assertEqual(response.status, 200)
        return response.read()

    def test_json_body_is_saved_without_changes(self):
        body = b'{ "resourceLogs": [] }\n'
        self.assertEqual(self.post("/v1/logs", body, "application/json"), b"{}")
        metadata = json.loads(next(self.output.glob("*.meta.json")).read_text())
        self.assertEqual(metadata["path"], "/v1/logs")
        self.assertEqual((self.output / metadata["body_file"]).read_bytes(), body)

    def test_chunked_protobuf_body_is_saved_without_changes(self):
        body = b"\x0a\x02\x12\x00\xff\x00"
        self.assertEqual(self.post("/v1/traces", iter([body[:2], body[2:]]),
                                   "application/x-protobuf", encode_chunked=True), b"")
        metadata = json.loads(next(self.output.glob("*.meta.json")).read_text())
        self.assertEqual((self.output / metadata["body_file"]).read_bytes(), body)

    def test_compressed_duplicate_requests_remain_separate(self):
        body = gzip.compress(b'{"resourceMetrics": []}', mtime=0)
        for _ in range(2):
            connection = http.client.HTTPConnection("127.0.0.1", self.address["port"], timeout=5)
            self.addCleanup(connection.close)
            connection.request("POST", "/v1/metrics", body, {
                "Content-Type": "application/json", "Content-Encoding": "gzip",
                "Authorization": "Bearer synthetic-do-not-save",
            })
            response = connection.getresponse()
            self.assertEqual(response.status, 200)
            response.read()
        captures = list(self.output.glob("*.meta.json"))
        self.assertEqual(len(captures), 2)
        for capture in captures:
            metadata = json.loads(capture.read_text())
            self.assertEqual(metadata["content_encoding"], "gzip")
            self.assertNotIn("synthetic-do-not-save", capture.read_text())
            self.assertEqual((self.output / metadata["body_file"]).read_bytes(), body)


if __name__ == "__main__":
    unittest.main()
