# ACOS SDKs

Official minimal clients for the versioned ACOS Computer API described by
[`spec/openapi/v1.yaml`](../spec/openapi/v1.yaml).

| SDK | Runtime | Runtime dependencies | Install / test |
| --- | --- | --- | --- |
| [Python](python/) | Python 3.10+ | None (standard library) | `python -m pip install ./sdk/python` then `cd sdk/python && python -m unittest discover -s tests` |
| [TypeScript](typescript/) | Node.js 22+ | None (built-in `fetch`) | `cd sdk/typescript && npm install && npm test && npm run typecheck` |

Both clients accept a base URL and bearer token, expose list/create/get/start/stop/pause/resume,
delete, command execution, guest information and workspace file-transfer operations, and preserve
structured API errors. Command output is base64 encoded as specified by the API. See each SDK's
README and `examples/` directory for usage.
