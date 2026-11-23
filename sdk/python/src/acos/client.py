"""HTTP client for the ACOS Computer API v1."""

from __future__ import annotations

import json
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.parse import quote
from urllib.request import Request, urlopen

from .types import APIErrorBody, CommandRequest, CommandResult, Computer, ComputerKind, CreateComputerRequest, Resources


class ACOSError(Exception):
    """An HTTP or API error returned while calling ACOS."""

    def __init__(
        self,
        message: str,
        *,
        status: int | None = None,
        code: str | None = None,
        request_id: str | None = None,
    ) -> None:
        super().__init__(message)
        self.status = status
        self.code = code
        self.request_id = request_id


class ACOSClient:
    """Small synchronous ACOS API client using only the Python standard library."""

    def __init__(
        self,
        base_url: str = "http://127.0.0.1:8787/v1",
        *,
        token: str,
        timeout: float = 30.0,
    ) -> None:
        if not token:
            raise ValueError("token must not be empty")
        self.base_url = base_url.rstrip("/")
        self.token = token
        self.timeout = timeout

    def __enter__(self) -> ACOSClient:
        return self

    def __exit__(self, *_: object) -> None:
        return None

    def list_computers(self) -> list[Computer]:
        """Return computers visible to the authenticated principal."""
        result = self._request("GET", "/computers")
        return result["items"]

    def create_computer(
        self,
        *,
        name: str,
        kind: ComputerKind,
        resources: Resources,
        image: str,
    ) -> Computer:
        """Submit a computer creation request."""
        body: CreateComputerRequest = {"name": name, "kind": kind, "resources": resources, "image": image}
        return self._request("POST", "/computers", body)

    def get_computer(self, computer_id: str) -> Computer:
        """Return the current observed state of a computer."""
        return self._request("GET", self._computer_path(computer_id))

    def start_computer(self, computer_id: str) -> Computer:
        """Request that a computer starts."""
        return self._request("POST", f"{self._computer_path(computer_id)}/start")

    def stop_computer(self, computer_id: str) -> Computer:
        """Request that a computer stops."""
        return self._request("POST", f"{self._computer_path(computer_id)}/stop")

    def pause_computer(self, computer_id: str) -> Computer:
        """Pause a computer when its backend supports it."""
        return self._request("POST", f"{self._computer_path(computer_id)}/pause")

    def resume_computer(self, computer_id: str) -> Computer:
        """Resume a paused computer."""
        return self._request("POST", f"{self._computer_path(computer_id)}/resume")

    def execute_command(self, computer_id: str, request: CommandRequest) -> CommandResult:
        """Execute an argv command in the guest; stdout/stderr are base64 strings."""
        return self._request("POST", f"{self._computer_path(computer_id)}/commands", request)

    def get_guest_info(self, computer_id: str) -> dict[str, Any]:
        """Return guest OS, hostname, filesystem and QEMU Guest Agent information."""
        return self._request("GET", f"{self._computer_path(computer_id)}/agent")

    def read_file(self, computer_id: str, path: str) -> bytes:
        """Read a workspace-relative file from the guest."""
        encoded = "/".join(quote(part, safe="") for part in path.strip("/").split("/"))
        request = Request(
            f"{self.base_url}{self._computer_path(computer_id)}/files/{encoded}",
            headers={"Authorization": f"Bearer {self.token}", "Accept": "application/octet-stream"},
        )
        try:
            with urlopen(request, timeout=self.timeout) as response:
                return response.read()
        except HTTPError as error:
            self._raise_api_error(error.code, error.read())
        except URLError as error:
            raise ACOSError(f"Unable to reach ACOS API: {error.reason}") from error

    def write_file(self, computer_id: str, path: str, contents: bytes) -> None:
        """Write bytes to a workspace-relative guest file."""
        encoded = "/".join(quote(part, safe="") for part in path.strip("/").split("/"))
        request = Request(
            f"{self.base_url}{self._computer_path(computer_id)}/files/{encoded}",
            data=contents,
            headers={"Authorization": f"Bearer {self.token}", "Content-Type": "application/octet-stream"},
            method="PUT",
        )
        try:
            with urlopen(request, timeout=self.timeout):
                return None
        except HTTPError as error:
            self._raise_api_error(error.code, error.read())
        except URLError as error:
            raise ACOSError(f"Unable to reach ACOS API: {error.reason}") from error

    def delete_computer(self, computer_id: str) -> None:
        """Request deletion of a computer."""
        self._request("DELETE", self._computer_path(computer_id), expected_json=False)

    @staticmethod
    def _computer_path(computer_id: str) -> str:
        if not computer_id:
            raise ValueError("computer_id must not be empty")
        return f"/computers/{quote(computer_id, safe='')}"

    def _request(
        self,
        method: str,
        path: str,
        body: dict[str, Any] | None = None,
        *,
        expected_json: bool = True,
    ) -> Any:
        headers = {"Authorization": f"Bearer {self.token}", "Accept": "application/json"}
        data = None
        if body is not None:
            headers["Content-Type"] = "application/json"
            data = json.dumps(body).encode("utf-8")
        request = Request(f"{self.base_url}{path}", data=data, headers=headers, method=method)
        try:
            with urlopen(request, timeout=self.timeout) as response:
                payload = response.read()
        except HTTPError as error:
            payload = error.read()
            self._raise_api_error(error.code, payload)
        except URLError as error:
            raise ACOSError(f"Unable to reach ACOS API: {error.reason}") from error
        if not expected_json or not payload:
            return None
        return json.loads(payload)

    @staticmethod
    def _raise_api_error(status: int, payload: bytes) -> None:
        try:
            parsed: APIErrorBody = json.loads(payload)
            detail = parsed["error"]
            raise ACOSError(
                detail["message"],
                status=status,
                code=detail["code"],
                request_id=parsed.get("request_id"),
            )
        except (KeyError, TypeError, ValueError, json.JSONDecodeError):
            raise ACOSError(
                f"ACOS API returned HTTP {status}", status=status
            ) from None
