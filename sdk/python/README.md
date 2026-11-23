# ACOS Python SDK

Minimal, dependency-free Python client for the ACOS Computer API v1. Requires Python 3.10+.

```sh
python -m pip install ./sdk/python
```

```python
from acos import ACOSClient

with ACOSClient("http://127.0.0.1:8787/v1", token="your-token") as client:
    computer = client.create_computer(
        name="agent-workspace",
        kind="vm",
        resources={"vcpus": 2, "memory_mib": 2048},
        image="acos-runtime-aarch64.qcow2",
    )
    computer = client.start_computer(computer["id"])
    print(computer["state"])
```

The token is sent as `Authorization: Bearer <token>`. `base_url` defaults to
`http://127.0.0.1:8787/v1`; pass an HTTPS URL for remote deployments. The client exposes
`list_computers`, `create_computer`, `get_computer`, `start_computer`, `stop_computer`,
`pause_computer`, `resume_computer`, `execute_command`, `get_guest_info`, and `delete_computer`. Command stdout and
stderr are base64-encoded strings. API failures raise `ACOSError`, which includes the HTTP status and, when
provided by the server, the structured API error and request ID.

`read_file` and `write_file` transfer raw bytes using workspace-relative guest paths (for example
`src/main.py`). The host restricts file transfers to `/workspace` and limits each transfer to 10 MiB.

See [`examples/manage_computer.py`](examples/manage_computer.py) and run tests with
`python -m unittest discover -s tests` from this directory.
