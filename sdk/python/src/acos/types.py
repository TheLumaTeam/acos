"""Types matching the public ACOS OpenAPI v1 schema."""

from typing import Literal, TypedDict

ComputerKind = Literal["vm", "container"]
ComputerState = Literal[
    "creating", "stopped", "starting", "running", "stopping", "paused", "failed", "unknown"
]


class Resources(TypedDict):
    vcpus: int
    memory_mib: int


class Computer(TypedDict):
    id: str
    name: str
    kind: ComputerKind
    state: ComputerState
    resources: Resources
    backend: str


class CreateComputerRequest(TypedDict):
    name: str
    kind: ComputerKind
    resources: Resources
    image: str


class CommandRequest(TypedDict, total=False):
    argv: list[str]
    working_directory: str
    environment: dict[str, str]
    timeout_ms: int
    output_limit_bytes: int


class CommandResult(TypedDict):
    exit_code: int
    stdout: str
    stderr: str
    duration_ms: int
    truncated: bool


class APIErrorDetail(TypedDict):
    code: str
    message: str


class APIErrorBody(TypedDict):
    error: APIErrorDetail
    request_id: str
