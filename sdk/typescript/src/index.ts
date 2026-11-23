export type ComputerKind = "vm" | "container";

export type ComputerState =
  | "creating"
  | "stopped"
  | "starting"
  | "running"
  | "stopping"
  | "paused"
  | "failed"
  | "unknown";

export interface Resources {
  vcpus: number;
  memory_mib: number;
}

export interface Computer {
  id: string;
  name: string;
  kind: ComputerKind;
  state: ComputerState;
  resources: Resources;
  backend: string;
}

export interface CreateComputerRequest {
  name: string;
  kind: ComputerKind;
  resources: Resources;
  image: string;
}

export interface CommandRequest {
  argv: string[];
  working_directory?: string;
  environment?: Record<string, string>;
  timeout_ms?: number;
  output_limit_bytes?: number;
}

export interface CommandResult {
  exit_code: number;
  stdout: string;
  stderr: string;
  duration_ms: number;
  truncated: boolean;
}

export interface ACOSClientOptions {
  baseUrl?: string;
  token: string;
  timeoutMs?: number;
}

interface APIErrorBody {
  error?: { code?: string; message?: string };
  request_id?: string;
}

export class ACOSError extends Error {
  public readonly status: number;
  public readonly code?: string;
  public readonly requestId?: string;

  constructor(
    message: string,
    status: number,
    code?: string,
    requestId?: string,
  ) {
    super(message);
    this.name = "ACOSError";
    this.status = status;
    this.code = code;
    this.requestId = requestId;
  }
}

/** Minimal client for the ACOS Computer API v1. */
export class ACOSClient {
  private readonly baseUrl: string;
  private readonly token: string;
  private readonly timeoutMs: number;

  constructor(options: ACOSClientOptions) {
    if (!options.token) throw new TypeError("token must not be empty");
    this.baseUrl = (options.baseUrl ?? "http://127.0.0.1:8787/v1").replace(/\/+$/, "");
    this.token = options.token;
    this.timeoutMs = options.timeoutMs ?? 30_000;
  }

  async listComputers(): Promise<Computer[]> {
    const result = await this.request<{ items: Computer[] }>("GET", "/computers");
    return result.items;
  }

  createComputer(input: CreateComputerRequest): Promise<Computer> {
    return this.request("POST", "/computers", input);
  }

  getComputer(computerId: string): Promise<Computer> {
    return this.request("GET", this.computerPath(computerId));
  }

  startComputer(computerId: string): Promise<Computer> {
    return this.request("POST", `${this.computerPath(computerId)}/start`);
  }

  stopComputer(computerId: string): Promise<Computer> {
    return this.request("POST", `${this.computerPath(computerId)}/stop`);
  }

  pauseComputer(computerId: string): Promise<Computer> {
    return this.request("POST", `${this.computerPath(computerId)}/pause`);
  }

  resumeComputer(computerId: string): Promise<Computer> {
    return this.request("POST", `${this.computerPath(computerId)}/resume`);
  }

  executeCommand(computerId: string, input: CommandRequest): Promise<CommandResult> {
    return this.request("POST", `${this.computerPath(computerId)}/commands`, input);
  }

  getGuestInfo(computerId: string): Promise<Record<string, unknown>> {
    return this.request("GET", `${this.computerPath(computerId)}/agent`);
  }

  async readFile(computerId: string, path: string): Promise<Uint8Array> {
    const response = await this.requestBinary("GET", `${this.computerPath(computerId)}/files/${this.filePath(path)}`);
    return new Uint8Array(response);
  }

  async writeFile(computerId: string, path: string, contents: Uint8Array): Promise<void> {
    await this.requestBinary("PUT", `${this.computerPath(computerId)}/files/${this.filePath(path)}`, contents);
  }

  private filePath(path: string): string {
    if (!path || path.startsWith("/") || path.split("/").some((part) => !part || part === "." || part === "..")) {
      throw new TypeError("path must be a workspace-relative path without empty, '.' or '..' segments");
    }
    return path.split("/").map(encodeURIComponent).join("/");
  }

  private async requestBinary(method: string, path: string, body?: Uint8Array): Promise<ArrayBuffer> {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), this.timeoutMs);
    try {
      const response = await fetch(`${this.baseUrl}${path}`, {
        method,
        headers: {
          Authorization: `Bearer ${this.token}`,
          "Content-Type": "application/octet-stream",
          Accept: "application/octet-stream",
        },
        body: body ? (body.buffer.slice(body.byteOffset, body.byteOffset + body.byteLength) as ArrayBuffer) : undefined,
        signal: controller.signal,
      });
      if (!response.ok) {
        const payload = (await response.json().catch(() => ({}))) as APIErrorBody;
        throw new ACOSError(payload.error?.message ?? `ACOS API returned HTTP ${response.status}`, response.status, payload.error?.code, payload.request_id);
      }
      return response.arrayBuffer();
    } catch (error) {
      if (controller.signal.aborted) throw new Error(`ACOS request timed out after ${this.timeoutMs} ms`);
      throw error;
    } finally {
      clearTimeout(timeout);
    }
  }

  async deleteComputer(computerId: string): Promise<void> {
    await this.request<void>("DELETE", this.computerPath(computerId), undefined, false);
  }

  private computerPath(computerId: string): string {
    if (!computerId) throw new TypeError("computerId must not be empty");
    return `/computers/${encodeURIComponent(computerId)}`;
  }

  private async request<T>(
    method: string,
    path: string,
    body?: unknown,
    parseJson = true,
  ): Promise<T> {
    const headers: Record<string, string> = {
      Authorization: `Bearer ${this.token}`,
      Accept: "application/json",
    };
    if (body !== undefined) headers["Content-Type"] = "application/json";

    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), this.timeoutMs);
    try {
      const response = await fetch(`${this.baseUrl}${path}`, {
        method,
        headers,
        body: body === undefined ? undefined : JSON.stringify(body),
        signal: controller.signal,
      });
      if (!response.ok) {
        const payload = (await response.json().catch(() => ({}))) as APIErrorBody;
        throw new ACOSError(
          payload.error?.message ?? `ACOS API returned HTTP ${response.status}`,
          response.status,
          payload.error?.code,
          payload.request_id,
        );
      }
      if (!parseJson || response.status === 204) return undefined as T;
      return (await response.json()) as T;
    } catch (error) {
      if (controller.signal.aborted) throw new Error(`ACOS request timed out after ${this.timeoutMs} ms`);
      throw error;
    } finally {
      clearTimeout(timeout);
    }
  }
}
