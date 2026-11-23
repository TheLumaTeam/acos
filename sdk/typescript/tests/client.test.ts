import assert from "node:assert/strict";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { after, before, test } from "node:test";
import { ACOSClient, ACOSError } from "../src/index.ts";

const computer = {
  id: "computer-1",
  name: "test",
  kind: "vm" as const,
  state: "running" as const,
  resources: { vcpus: 2, memory_mib: 2048 },
  backend: "qemu",
};

const requests: Array<{ method: string; url: string; authorization?: string; body: string }> = [];
let client: ACOSClient;
let server: ReturnType<typeof createServer>;
let baseUrl: string;

function respond(response: ServerResponse, status: number, value?: unknown): void {
  response.writeHead(status, value === undefined ? {} : { "content-type": "application/json" });
  response.end(value === undefined ? undefined : JSON.stringify(value));
}

before(async () => {
  server = createServer(async (request: IncomingMessage, response: ServerResponse) => {
    const chunks: Buffer[] = [];
    for await (const chunk of request) chunks.push(Buffer.from(chunk));
    const body = Buffer.concat(chunks).toString();
    requests.push({
      method: request.method ?? "",
      url: request.url ?? "",
      authorization: request.headers.authorization,
      body,
    });
    if (request.url === "/v1/computers/missing") {
      respond(response, 404, {
        error: { code: "not_found", message: "Missing" },
        request_id: "req-1",
      });
    } else if (request.method === "GET" && request.url?.includes("/files/")) {
      response.writeHead(200, { "content-type": "application/octet-stream" });
      response.end(Buffer.from("hello"));
    } else if (request.method === "GET" && request.url === "/v1/computers") {
      respond(response, 200, { items: [computer] });
    } else if (request.method === "DELETE") {
      respond(response, 202);
    } else {
      respond(response, 202, computer);
    }
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  if (!address || typeof address === "string") throw new Error("Test server did not bind TCP");
  baseUrl = `http://127.0.0.1:${address.port}/v1`;
  client = new ACOSClient({ baseUrl, token: "secret" });
});

after(async () => {
  await new Promise<void>((resolve, reject) => server.close((error) => (error ? reject(error) : resolve())));
});

test("covers lifecycle, guest commands, guest info, file transfer and bearer auth", async () => {
  assert.deepEqual(await client.listComputers(), [computer]);
  assert.equal(
    (await client.createComputer({
      name: "test",
      kind: "vm",
      resources: { vcpus: 2, memory_mib: 2048 },
      image: "base.qcow2",
    })).id,
    computer.id,
  );
  await client.getComputer(computer.id);
  await client.startComputer(computer.id);
  await client.stopComputer(computer.id);
  await client.pauseComputer(computer.id);
  await client.resumeComputer(computer.id);
  await client.executeCommand(computer.id, { argv: ["python3", "--version"] });
  await client.getGuestInfo(computer.id);
  assert.equal(Buffer.from(await client.readFile(computer.id, "src/main.py")).toString(), "hello");
  await client.writeFile(computer.id, "src/main.py", new TextEncoder().encode("updated"));
  await client.deleteComputer(computer.id);
  assert.deepEqual(requests.map(({ method, url }) => [method, url]), [
    ["GET", "/v1/computers"],
    ["POST", "/v1/computers"],
    ["GET", "/v1/computers/computer-1"],
    ["POST", "/v1/computers/computer-1/start"],
    ["POST", "/v1/computers/computer-1/stop"],
    ["POST", "/v1/computers/computer-1/pause"],
    ["POST", "/v1/computers/computer-1/resume"],
    ["POST", "/v1/computers/computer-1/commands"],
    ["GET", "/v1/computers/computer-1/agent"],
    ["GET", "/v1/computers/computer-1/files/src/main.py"],
    ["PUT", "/v1/computers/computer-1/files/src/main.py"],
    ["DELETE", "/v1/computers/computer-1"],
  ]);
  assert.ok(requests.every((request) => request.authorization === "Bearer secret"));
  assert.equal(JSON.parse(requests[1]!.body).resources.memory_mib, 2048);
});

test("preserves structured API errors", async () => {
  await assert.rejects(client.getComputer("missing"), (error: unknown) => {
    assert.ok(error instanceof ACOSError);
    assert.equal(error.status, 404);
    assert.equal(error.code, "not_found");
    assert.equal(error.requestId, "req-1");
    return true;
  });
});

test("rejects an empty token", () => {
  assert.throws(() => new ACOSClient({ baseUrl, token: "" }), TypeError);
});
