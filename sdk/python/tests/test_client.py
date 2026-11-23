"""HTTP-level tests for the ACOS Python SDK."""

import json
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from acos import ACOSClient, ACOSError


class Handler(BaseHTTPRequestHandler):
    requests: list[tuple[str, str, str, bytes]] = []

    def do_GET(self) -> None:
        if "/files/" in self.path:
            self.requests.append(("GET", self.path, self.headers.get("Authorization", ""), b""))
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.end_headers()
            self.wfile.write(b"hello")
            return
        self._respond(200, {"items": [COMPUTER]} if self.path == "/v1/computers" else COMPUTER)

    def do_PUT(self) -> None:
        body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        self.requests.append(("PUT", self.path, self.headers.get("Authorization", ""), body))
        self.send_response(204)
        self.end_headers()

    def do_POST(self) -> None:
        body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        self._respond(202, COMPUTER, body)

    def do_DELETE(self) -> None:
        self.requests.append(("DELETE", self.path, self.headers.get("Authorization", ""), b""))
        self.send_response(202)
        self.end_headers()

    def _respond(self, status: int, value: dict, body: bytes = b"") -> None:
        self.requests.append((self.command, self.path, self.headers.get("Authorization", ""), body))
        payload = json.dumps(value).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, *_: object) -> None:
        pass


COMPUTER = {
    "id": "computer-1",
    "name": "test",
    "kind": "vm",
    "state": "running",
    "resources": {"vcpus": 2, "memory_mib": 2048},
    "backend": "qemu",
}


class ACOSClientTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        Handler.requests = []
        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.client = ACOSClient(f"http://127.0.0.1:{cls.server.server_port}/v1", token="secret")

    @classmethod
    def tearDownClass(cls) -> None:
        cls.server.shutdown()
        cls.server.server_close()
        cls.thread.join()

    def test_lifecycle_methods_and_bearer_auth(self) -> None:
        self.assertEqual(self.client.list_computers(), [COMPUTER])
        created = self.client.create_computer(
            name="test", kind="vm", resources={"vcpus": 2, "memory_mib": 2048}, image="base.qcow2"
        )
        self.assertEqual(created["id"], "computer-1")
        self.client.get_computer("computer-1")
        self.client.start_computer("computer-1")
        self.client.stop_computer("computer-1")
        self.client.pause_computer("computer-1")
        self.client.resume_computer("computer-1")
        self.client.execute_command("computer-1", {"argv": ["python3", "--version"]})
        self.client.get_guest_info("computer-1")
        self.client.delete_computer("computer-1")
        self.assertEqual(
            [request[1] for request in Handler.requests],
            [
                "/v1/computers",
                "/v1/computers",
                "/v1/computers/computer-1",
                "/v1/computers/computer-1/start",
                "/v1/computers/computer-1/stop",
                "/v1/computers/computer-1/pause",
                "/v1/computers/computer-1/resume",
                "/v1/computers/computer-1/commands",
                "/v1/computers/computer-1/agent",
                "/v1/computers/computer-1",
            ],
        )
        self.assertTrue(all(request[2] == "Bearer secret" for request in Handler.requests))
        create_body = json.loads(Handler.requests[1][3])
        self.assertEqual(create_body["resources"]["memory_mib"], 2048)

    def test_empty_token_is_rejected(self) -> None:
        with self.assertRaises(ValueError):
            ACOSClient(token="")

    def test_workspace_file_transfer(self) -> None:
        self.assertEqual(self.client.read_file("computer-1", "src/main.py"), b"hello")
        self.client.write_file("computer-1", "src/main.py", b"updated")
        self.assertEqual(Handler.requests[-2][1], "/v1/computers/computer-1/files/src/main.py")
        self.assertEqual(Handler.requests[-1][3], b"updated")

    def test_api_error_fields_are_preserved(self) -> None:
        from unittest.mock import patch
        from urllib.error import HTTPError
        from io import BytesIO

        error_body = json.dumps(
            {"error": {"code": "not_found", "message": "Missing"}, "request_id": "req-1"}
        ).encode()
        error = HTTPError("http://localhost", 404, "Not Found", {}, BytesIO(error_body))
        with (
            patch("acos.client.urlopen", side_effect=error),
            self.assertRaises(ACOSError) as caught,
        ):
            self.client.get_computer("missing")
        self.assertEqual(caught.exception.status, 404)
        self.assertEqual(caught.exception.code, "not_found")
        self.assertEqual(caught.exception.request_id, "req-1")


if __name__ == "__main__":
    unittest.main()
