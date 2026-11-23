# ACOS TypeScript SDK

Minimal TypeScript client for ACOS Computer API v1. Requires Node.js 22+ and uses the built-in
`fetch` API; there are no runtime dependencies.

```sh
npm install
npm run build
npm run typecheck
npm test
```

```ts
import { ACOSClient } from "@acos/sdk";

const client = new ACOSClient({ baseUrl: "http://127.0.0.1:8787/v1", token: "your-token" });
const computer = await client.createComputer({
  name: "agent-workspace",
  kind: "vm",
  resources: { vcpus: 2, memory_mib: 2048 },
  image: "acos-runtime-aarch64.qcow2",
});
console.log((await client.startComputer(computer.id)).state);
```

The token is sent as `Authorization: Bearer <token>`. The base URL defaults to
`http://127.0.0.1:8787/v1`; use HTTPS for remote deployments. Methods are `listComputers`,
`createComputer`, `getComputer`, `startComputer`, `stopComputer`, `pauseComputer`,
`resumeComputer`, `executeCommand`, `getGuestInfo`, and `deleteComputer`. Command stdout and stderr are
base64-encoded strings.
`readFile` and `writeFile` transfer raw bytes using workspace-relative paths and a 10 MiB transfer limit.
Non-success responses throw `ACOSError`, preserving the HTTP status and structured API error
code, message, and request ID when supplied.

See [`examples/manage-computer.ts`](examples/manage-computer.ts) for a complete lifecycle example.
