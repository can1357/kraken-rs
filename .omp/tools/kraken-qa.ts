import { mkdir } from "node:fs/promises";
import { createConnection, type Socket } from "node:net";
import { dirname, resolve } from "node:path";

import type { CustomToolFactory } from "@oh-my-pi/pi-coding-agent";

interface ProcessHandle {
  readonly pid: number;
  readonly exited: Promise<number>;
  readonly exitCode: number | null;
  readonly stderr: ReadableStream<Uint8Array>;
  kill(signal?: number | string): void;
}

interface QaSession {
  child: ProcessHandle;
  sdp: SdpClient;
  host: string;
  port: number;
  width: number;
  height: number;
  stderr: string[];
}

interface Endpoint {
  host: string;
  port: number;
}

interface RpcError {
  code?: number;
  message: string;
}

interface RpcResponse {
  id: number;
  result?: unknown;
  error?: RpcError;
}

interface PendingRequest {
  method: string;
  resolve(value: unknown): void;
  reject(error: Error): void;
}

let active: QaSession | undefined;

const factory: CustomToolFactory = (pi) => ({
  name: "kraken_qa",
  label: "Kraken QA",
  description:
    "Drives Kraken Native through the Slab Drive Protocol (SDP): launch, inspect the semantic scene tree, render PNGs, send standard input events, resize the environment, issue raw SDP requests, and terminate.",
  parameters: pi.zod
    .object({
      operation: pi.zod.enum([
        "launch",
        "snapshot",
        "screenshot",
        "click",
        "move",
        "type",
        "key",
        "scroll",
        "viewport",
        "command",
        "close",
      ]),
      repo: pi.zod.string().optional(),
      executable: pi.zod.string().optional(),
      build: pi.zod.boolean().optional().default(true),
      width: pi.zod.number().int().min(640).max(7680).optional(),
      height: pi.zod.number().int().min(480).max(4320).optional(),
      path: pi.zod.string().optional(),
      key: pi.zod.string().optional(),
      x: pi.zod.number().finite().optional(),
      y: pi.zod.number().finite().optional(),
      deltaX: pi.zod.number().finite().optional(),
      deltaY: pi.zod.number().finite().optional(),
      text: pi.zod.string().optional(),
      command: pi.zod.boolean().optional(),
      control: pi.zod.boolean().optional(),
      alt: pi.zod.boolean().optional(),
      shift: pi.zod.boolean().optional(),
      timeoutMs: pi.zod.number().int().positive().max(120_000).optional(),
      method: pi.zod.string().optional(),
      params: pi.zod.record(pi.zod.string(), pi.zod.unknown()).optional(),
      args: pi.zod.array(pi.zod.string()).optional(),
    })
    .strict(),

  async execute(_toolCallId, params, onUpdate, _ctx, signal) {
    switch (params.operation) {
      case "launch": {
        const running = active;
        if (running?.child.exitCode === null) {
          return result("Kraken QA is already running", {
            pid: running.child.pid,
            host: running.host,
            port: running.port,
            width: running.width,
            height: running.height,
          });
        }
        const executable = resolve(pi.cwd, params.executable ?? "target/debug/kraken");
        if (params.build) {
          onUpdate?.({
            content: [{ type: "text", text: "Building Kraken Native…" }],
            details: { phase: "build" },
          });
          const build = await pi.exec("cargo", ["build"], { cwd: pi.cwd, signal });
          if (build.killed) {
            throw new Error("Kraken build was cancelled");
          }
          if (build.code !== 0) {
            throw new Error(build.stderr || "Kraken build failed");
          }
        } else if (!(await Bun.file(executable).exists())) {
          throw new Error(`Kraken executable does not exist: ${executable}`);
        }

        const repo = resolve(pi.cwd, params.repo ?? pi.cwd);
        const width = params.width ?? 1600;
        const height = params.height ?? 900;
        const stderr: string[] = [];
        const child = Bun.spawn({
          cmd: [
            executable,
            "--repo",
            repo,
            "--drive-port",
            "0",
            "--width",
            String(width),
            "--height",
            String(height),
            ...(params.args ?? []),
          ],
          cwd: pi.cwd,
          env: process.env,
          stdin: "ignore",
          stdout: "ignore",
          stderr: "pipe",
        });
        const timeoutMs = params.timeoutMs ?? 30_000;
        let endpoint: Endpoint;
        let sdp: SdpClient | undefined;
        try {
          endpoint = await readyEndpoint(child.stderr, stderr, timeoutMs, signal);
          const connected = await SdpClient.connect(endpoint, timeoutMs, signal);
          sdp = connected;
          const protocol = await withTimeout(
            connected.request("protocol.info", {}),
            timeoutMs,
            "protocol.info",
          );
          if (
            typeof protocol !== "object" ||
            protocol === null ||
            Array.isArray(protocol) ||
            !(("name" in protocol) && ("version" in protocol)) ||
            protocol.name !== "sdp" ||
            protocol.version !== 1
          ) {
            throw new Error("Kraken did not expose Slab Drive Protocol version 1");
          }
          const session: QaSession = {
            child,
            sdp: connected,
            host: endpoint.host,
            port: endpoint.port,
            width,
            height,
            stderr,
          };
          active = session;
          child.exited.then(() => {
            if (active?.child === child) {
              active.sdp.close();
              active = undefined;
            }
          });
          return result(
            `Launched Kraken SDP on ${endpoint.host}:${endpoint.port}`,
            {
              pid: session.child.pid,
              host: session.host,
              port: session.port,
              width: session.width,
              height: session.height,
              protocol,
            },
          );
        } catch (error) {
          sdp?.close();
          child.kill("SIGTERM");
          await child.exited;
          const diagnostic = stderr.length > 0 ? `\n${stderr.join("\n")}` : "";
          throw new Error(`${message(error)}${diagnostic}`);
        }
      }

      case "snapshot": {
        const session = requireSession();
        const snapshot = await sdp(session, "scene.tree", {}, params.timeoutMs);
        return result(snapshotText(snapshot), snapshot);
      }

      case "screenshot": {
        const session = requireSession();
        const output = resolve(
          pi.cwd,
          params.path ?? `.omp/qa/kraken-${Date.now()}.png`,
        );
        await mkdir(dirname(output), { recursive: true });
        const capture = await sdp(
          session,
          "render.png",
          { path: output },
          params.timeoutMs,
        );
        const file = Bun.file(output);
        if (!(await file.exists())) {
          throw new Error(`Kraken did not create screenshot: ${output}`);
        }
        const data = Buffer.from(await file.arrayBuffer()).toString("base64");
        return {
          content: [
            { type: "text", text: `Captured ${output}` },
            { type: "image", data, mimeType: "image/png" },
          ],
          details: { operation: params.operation, capture },
        };
      }

      case "click": {
        const session = requireSession();
        const input: Record<string, unknown> = { mods: modifiers(params) };
        if (params.key !== undefined) {
          input.key = params.key;
        } else {
          input.x = required(params.x, "x");
          input.y = required(params.y, "y");
        }
        const clicked = await sdp(session, "input.click", input, params.timeoutMs);
        return result(`Clicked ${params.key ?? "coordinates"}`, clicked);
      }

      case "move": {
        const session = requireSession();
        const moved = await sdp(
          session,
          "input.pointer",
          {
            type: "move",
            x: required(params.x, "x"),
            y: required(params.y, "y"),
            mods: modifiers(params),
          },
          params.timeoutMs,
        );
        return result(`Moved pointer to ${params.x}, ${params.y}`, moved);
      }

      case "type": {
        const session = requireSession();
        const text = required(params.text, "text");
        const typed = await sdp(session, "input.text", { text }, params.timeoutMs);
        return result(`Inserted ${text.length} character(s)`, typed);
      }

      case "key": {
        const session = requireSession();
        const key = required(params.key, "key");
        const pressed = await sdp(
          session,
          "input.key",
          { key, mods: modifiers(params) },
          params.timeoutMs,
        );
        return result(`Pressed ${key}`, pressed);
      }

      case "scroll": {
        const session = requireSession();
        const scrolled = await sdp(
          session,
          "input.wheel",
          {
            x: params.x ?? session.width * 0.5,
            y: params.y ?? session.height * 0.5,
            dx: params.deltaX ?? 0,
            dy: params.deltaY ?? 480,
            mods: modifiers(params),
          },
          params.timeoutMs,
        );
        return result(`Scrolled by ${params.deltaY ?? 480} px`, scrolled);
      }

      case "viewport": {
        const session = requireSession();
        const width = required(params.width, "width");
        const height = required(params.height, "height");
        const viewport = await sdp(
          session,
          "env.set",
          { width, height },
          params.timeoutMs,
        );
        session.width = width;
        session.height = height;
        return result(`Set viewport to ${width}×${height}`, viewport);
      }

      case "command": {
        const session = requireSession();
        const method = required(params.method, "method");
        const response = await sdp(
          session,
          method,
          params.params ?? {},
          params.timeoutMs,
        );
        return result(JSON.stringify(response, null, 2), response);
      }

      case "close": {
        const session = requireSession();
        const closed = await closeSession(session, params.timeoutMs ?? 5_000);
        return result("Terminated Kraken SDP session", closed);
      }
    }
  },

  onSession(event) {
    if (event.reason === "shutdown" && active !== undefined) {
      void closeSession(active, 2_000);
    }
  },
});

class SdpRemoteError extends Error {
  constructor(method: string, code: number | undefined, detail: string) {
    super(`${method}: ${detail}`);
    this.name = "SdpRemoteError";
    this.code = code;
  }

  readonly code: number | undefined;
}

class SdpClient {
  private readonly socket: Socket;
  private readonly pending = new Map<number, PendingRequest>();
  private buffered = "";
  private nextRequestId = 1;
  private failure: Error | undefined;
  private closed = false;

  private constructor(socket: Socket) {
    this.socket = socket;
    socket.setEncoding("utf8");
    socket.on("data", (chunk) => this.receive(chunk.toString()));
    socket.once("end", () => this.fail(new Error("SDP transport closed before responding")));
    socket.once("close", () => this.fail(new Error("SDP transport closed before responding")));
    socket.once("error", (error) => this.fail(error));
  }

  static async connect(
    endpoint: Endpoint,
    timeoutMs: number,
    signal: AbortSignal,
  ): Promise<SdpClient> {
    if (signal.aborted) {
      throw new Error("Kraken launch was cancelled");
    }
    const socket = createConnection({ host: endpoint.host, port: endpoint.port });
    const connection = Promise.withResolvers<SdpClient>();
    const cancel = () => fail(new Error("Kraken launch was cancelled"));
    const fail = (error: Error) => {
      socket.destroy();
      connection.reject(error);
    };
    socket.once("error", fail);
    socket.once("connect", () => {
      socket.off("error", fail);
      connection.resolve(new SdpClient(socket));
    });
    signal.addEventListener("abort", cancel, { once: true });
    try {
      return await withTimeout(connection.promise, timeoutMs, "SDP connection");
    } catch (error) {
      socket.destroy();
      throw error;
    } finally {
      signal.removeEventListener("abort", cancel);
    }
  }

  request(method: string, params: Record<string, unknown>): Promise<unknown> {
    if (this.closed) {
      return Promise.reject(new Error("SDP transport is closed"));
    }
    if (this.failure !== undefined) {
      return Promise.reject(this.failure);
    }
    if (this.nextRequestId > Number.MAX_SAFE_INTEGER) {
      return Promise.reject(new Error("SDP request id space is exhausted"));
    }
    const id = this.nextRequestId;
    this.nextRequestId += 1;
    let line: string | undefined;
    try {
      line = JSON.stringify({ id, method, params });
    } catch (error) {
      return Promise.reject(new Error(`Cannot encode ${method}: ${message(error)}`));
    }
    if (line === undefined) {
      return Promise.reject(new Error(`Cannot encode ${method}`));
    }
    const response = Promise.withResolvers<unknown>();
    this.pending.set(id, { method, ...response });
    this.socket.write(`${line}\n`, "utf8", (error) => {
      if (error !== null && error !== undefined) {
        this.fail(error);
      }
    });
    return response.promise;
  }

  close(error = new Error("SDP client closed")): void {
    if (this.closed) {
      return;
    }
    this.closed = true;
    this.fail(error);
    this.socket.destroy();
  }

  private receive(chunk: string): void {
    this.buffered += chunk;
    let newline = this.buffered.indexOf("\n");
    while (newline >= 0) {
      const line = this.buffered.slice(0, newline).replace(/\r$/, "");
      this.buffered = this.buffered.slice(newline + 1);
      if (line.trim().length > 0) {
        this.handleLine(line);
      }
      newline = this.buffered.indexOf("\n");
    }
  }

  private handleLine(line: string): void {
    let response: unknown;
    try {
      response = JSON.parse(line);
    } catch (error) {
      this.fail(new Error(`Invalid SDP response: ${message(error)}`));
      return;
    }
    if (!isRpcResponse(response)) {
      this.fail(new Error("Malformed SDP response"));
      return;
    }
    const pending = this.pending.get(response.id);
    if (pending === undefined) {
      this.fail(new Error(`SDP response has no pending request for id ${response.id}`));
      return;
    }
    this.pending.delete(response.id);
    if (response.error !== undefined) {
      pending.reject(new SdpRemoteError(pending.method, response.error.code, response.error.message));
      return;
    }
    pending.resolve(response.result);
  }

  private fail(error: Error): void {
    if (this.failure !== undefined) {
      return;
    }
    this.failure = error;
    for (const pending of this.pending.values()) {
      pending.reject(error);
    }
    this.pending.clear();
  }
}

async function readyEndpoint(
  stream: ReadableStream<Uint8Array>,
  lines: string[],
  timeoutMs: number,
  signal: AbortSignal,
): Promise<Endpoint> {
  let ready = false;
  const endpoint = Promise.withResolvers<Endpoint>();
  void drain(stream, lines, (line) => {
    if (ready) {
      return;
    }
    const match = /^sdp: listening on (127\.0\.0\.1):(\d+)$/.exec(line.trim());
    if (match === null) {
      return;
    }
    const port = Number(match[2]);
    if (!Number.isSafeInteger(port) || port < 1 || port > 65_535) {
      ready = true;
      endpoint.reject(new Error("Kraken published an invalid SDP endpoint"));
      return;
    }
    ready = true;
    endpoint.resolve({ host: match[1], port });
  }).then(
    () => {
      if (!ready) {
        endpoint.reject(new Error("Kraken exited before publishing its SDP endpoint"));
      }
    },
    (error) => endpoint.reject(error instanceof Error ? error : new Error(message(error))),
  );
  return withAbort(
    withTimeout(endpoint.promise, timeoutMs, "SDP endpoint"),
    signal,
    "Kraken launch was cancelled",
  );
}

async function sdp(
  session: QaSession,
  method: string,
  params: Record<string, unknown>,
  timeoutMs = 15_000,
): Promise<unknown> {
  if (session.child.exitCode !== null) {
    const diagnostic = session.stderr.length > 0 ? `: ${session.stderr.join("\n")}` : "";
    throw new Error(`Headless Kraken has exited${diagnostic}`);
  }
  try {
    return await withTimeout(session.sdp.request(method, params), timeoutMs, method);
  } catch (error) {
    if (!(error instanceof SdpRemoteError)) {
      session.sdp.close(error instanceof Error ? error : new Error(message(error)));
    }
    throw error;
  }
}

async function closeSession(session: QaSession, timeoutMs: number): Promise<unknown> {
  let response: unknown;
  try {
    response = await sdp(session, "protocol.quit", {}, timeoutMs);
    await withTimeout(session.child.exited, timeoutMs, "Kraken termination");
  } catch (error) {
    if (session.child.exitCode === null) {
      session.child.kill("SIGTERM");
      await withTimeout(session.child.exited, timeoutMs, "forced Kraken termination");
    }
    response = { forced: true, reason: message(error) };
  } finally {
    session.sdp.close();
    if (active?.child === session.child) {
      active = undefined;
    }
  }
  return response;
}

async function drain(
  stream: ReadableStream<Uint8Array>,
  lines: string[],
  onLine?: (line: string) => void,
): Promise<void> {
  const reader = stream.getReader();
  const decoder = new TextDecoder();
  let buffered = "";
  const report = (line: string) => {
    lines.push(line);
    if (lines.length > 100) {
      lines.splice(0, lines.length - 100);
    }
    onLine?.(line);
  };
  while (true) {
    const chunk = await reader.read();
    if (chunk.done) {
      break;
    }
    buffered += decoder.decode(chunk.value, { stream: true });
    const complete = buffered.split("\n");
    buffered = complete.pop() ?? "";
    for (const line of complete) {
      if (line.length === 0) {
        continue;
      }
      report(line);
    }
  }
  if (buffered.length > 0) {
    report(buffered);
  }
}


async function withTimeout<T>(
  promise: Promise<T>,
  timeoutMs: number,
  label: string,
): Promise<T> {
  const timeout = Promise.withResolvers<T>();
  const timer = setTimeout(
    () => timeout.reject(new Error(`${label} timed out after ${timeoutMs} ms`)),
    timeoutMs,
  );
  try {
    return await Promise.race([promise, timeout.promise]);
  } finally {
    clearTimeout(timer);
  }
}

async function withAbort<T>(
  promise: Promise<T>,
  signal: AbortSignal,
  reason: string,
): Promise<T> {
  if (signal.aborted) {
    throw new Error(reason);
  }
  const cancellation = Promise.withResolvers<T>();
  const cancel = () => cancellation.reject(new Error(reason));
  signal.addEventListener("abort", cancel, { once: true });
  try {
    return await Promise.race([promise, cancellation.promise]);
  } finally {
    signal.removeEventListener("abort", cancel);
  }
}

function requireSession(): QaSession {
  if (active === undefined || active.child.exitCode !== null) {
    throw new Error("Kraken QA is not running; call launch first");
  }
  return active;
}

function required<T>(value: T | undefined, name: string): T {
  if (value === undefined) {
    throw new Error(`Operation requires \`${name}\``);
  }
  return value;
}

function modifiers(params: {
  command?: boolean;
  control?: boolean;
  alt?: boolean;
  shift?: boolean;
}): string[] {
  const modifiers: string[] = [];
  if (params.shift) {
    modifiers.push("shift");
  }
  if (params.alt) {
    modifiers.push("alt");
  }
  if (params.control) {
    modifiers.push("ctrl");
  }
  if (params.command) {
    modifiers.push("meta");
  }
  return modifiers;
}

function result(text: string, details: unknown) {
  return {
    content: [{ type: "text", text }],
    details: { details },
  };
}


function snapshotText(snapshot: unknown): string {
  if (
    typeof snapshot !== "object" ||
    snapshot === null ||
    Array.isArray(snapshot) ||
    !(("nodes" in snapshot) && Array.isArray(snapshot.nodes))
  ) {
    return JSON.stringify(snapshot, null, 2);
  }
  const details = snapshot.nodes.flatMap((node) => {
    if (typeof node !== "object" || node === null || Array.isArray(node)) {
      return [];
    }
    const key = "key" in node && typeof node.key === "string" ? node.key : "<unkeyed>";
    const role = "role" in node && typeof node.role === "string" ? node.role : "generic";
    const label = "label" in node && typeof node.label === "string" ? ` — ${node.label}` : "";
    return [`${key} [${role}]${label}`];
  });
  return [`Scene nodes (${snapshot.nodes.length}):`, ...details].join("\n");
}

function isRpcResponse(value: unknown): value is RpcResponse {
  if (
    typeof value !== "object" ||
    value === null ||
    Array.isArray(value) ||
    !("id" in value) ||
    typeof value.id !== "number"
  ) {
    return false;
  }
  if (!("error" in value) || value.error === undefined) {
    return true;
  }
  const error = value.error;
  return (
    typeof error === "object" &&
    error !== null &&
    !Array.isArray(error) &&
    "message" in error &&
    typeof error.message === "string" &&
    (!("code" in error) || error.code === undefined || typeof error.code === "number")
  );
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export default factory;
