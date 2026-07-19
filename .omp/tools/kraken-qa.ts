import { mkdir } from "node:fs/promises";
import { createConnection } from "node:net";
import { dirname, resolve } from "node:path";

import type { CustomToolFactory } from "@oh-my-pi/pi-coding-agent";


interface ProcessHandle {
  readonly pid: number;
  readonly exited: Promise<number>;
  readonly exitCode: number | null;
  readonly stdout: ReadableStream<Uint8Array>;
  readonly stderr: ReadableStream<Uint8Array>;
  kill(signal?: number | string): void;
}

interface QaSession {
  child: ProcessHandle;
  host: string;
  port: number;
  width: number;
  height: number;
  stderr: string[];
}

interface Endpoint {
  host: string;
  port: number;
  protocolVersion: string;
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

let active: QaSession | undefined;
let nextRequestId = 1;

const factory: CustomToolFactory = (pi) => ({
  name: "kraken_qa",
  label: "Kraken QA",
  description:
    "Drives Kraken Native through its headless CDP-like protocol: launch, inspect semantic UI, capture screenshots, click controls, type, press keys, scroll, resize, send raw protocol commands, and terminate.",
  parameters: pi.zod
    .object({
      operation: pi.zod.enum([
        "launch",
        "state",
        "snapshot",
        "screenshot",
        "click",
        "move",
        "type",
        "key",
        "scroll",
        "viewport",
        "wait",
        "command",
        "close",
      ]),
      repo: pi.zod.string().optional(),
      executable: pi.zod.string().optional(),
      build: pi.zod.boolean().optional().default(true),
      width: pi.zod.number().int().min(640).max(7680).optional(),
      height: pi.zod.number().int().min(480).max(4320).optional(),
      path: pi.zod.string().optional(),
      selector: pi.zod.string().optional(),
      x: pi.zod.number().finite().optional(),
      y: pi.zod.number().finite().optional(),
      deltaX: pi.zod.number().finite().optional(),
      deltaY: pi.zod.number().finite().optional(),
      text: pi.zod.string().optional(),
      key: pi.zod.string().optional(),
      command: pi.zod.boolean().optional(),
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
        if (active?.child.exitCode === null) {
          return result("Kraken QA is already running", sessionDetails(active));
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
            "--automation-port",
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
          stdout: "pipe",
          stderr: "pipe",
        });
        void drain(child.stderr, stderr);

        let endpoint: Endpoint;
        try {
          endpoint = await readyEndpoint(child.stdout, params.timeoutMs ?? 30_000, signal);
        } catch (error) {
          child.kill("SIGTERM");
          await child.exited;
          const diagnostic = stderr.length > 0 ? `\n${stderr.join("\n")}` : "";
          throw new Error(`${message(error)}${diagnostic}`);
        }

        active = {
          child,
          host: endpoint.host,
          port: endpoint.port,
          width,
          height,
          stderr,
        };
        child.exited.then(() => {
          if (active?.child === child) {
            active = undefined;
          }
        });
        const state = await rpc(active, "App.waitForIdle", {
          timeoutMs: params.timeoutMs ?? 30_000,
        });
        return result(
          `Launched headless Kraken on ${endpoint.host}:${endpoint.port}`,
          { ...sessionDetails(active), state },
        );
      }

      case "state": {
        const session = requireSession();
        const state = await rpc(session, "App.getState", {}, params.timeoutMs);
        return result(JSON.stringify(state, null, 2), state);
      }

      case "snapshot": {
        const session = requireSession();
        const snapshot = await rpc(session, "Page.getSnapshot", {}, params.timeoutMs);
        return result(snapshotText(snapshot), snapshot);
      }

      case "screenshot": {
        const session = requireSession();
        const output = resolve(
          pi.cwd,
          params.path ?? `.omp/qa/kraken-${Date.now()}.png`,
        );
        await mkdir(dirname(output), { recursive: true });
        const capture = await rpc(
          session,
          "Page.captureScreenshot",
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
        const clickParams: Record<string, unknown> = {
          command: params.command ?? false,
          shift: params.shift ?? false,
        };
        if (params.selector !== undefined) {
          clickParams.selector = params.selector;
        } else {
          clickParams.x = required(params.x, "x");
          clickParams.y = required(params.y, "y");
        }
        const clicked = await rpc(session, "UI.click", clickParams, params.timeoutMs);
        return result(`Clicked ${targetName(clicked)}`, clicked);
      }

      case "move": {
        const session = requireSession();
        const moved = await rpc(
          session,
          "Input.dispatchMouseEvent",
          {
            type: "mouseMoved",
            x: required(params.x, "x"),
            y: required(params.y, "y"),
          },
          params.timeoutMs,
        );
        return result(`Moved pointer to ${params.x}, ${params.y}`, moved);
      }

      case "type": {
        const session = requireSession();
        const text = required(params.text, "text");
        const typed = await rpc(
          session,
          "Input.insertText",
          { text },
          params.timeoutMs,
        );
        return result(`Inserted ${text.length} character(s)`, typed);
      }

      case "key": {
        const session = requireSession();
        const key = required(params.key, "key");
        const pressed = await rpc(
          session,
          "Input.dispatchKeyEvent",
          {
            type: "keyDown",
            key,
            text: params.text,
            command: params.command ?? false,
            shift: params.shift ?? false,
          },
          params.timeoutMs,
        );
        return result(`Pressed ${key}`, pressed);
      }

      case "scroll": {
        const session = requireSession();
        const scrolled = await rpc(
          session,
          "Input.dispatchMouseEvent",
          {
            type: "mouseWheel",
            x: params.x ?? session.width * 0.5,
            y: params.y ?? session.height * 0.5,
            deltaX: params.deltaX ?? 0,
            deltaY: params.deltaY ?? 480,
          },
          params.timeoutMs,
        );
        return result(`Scrolled by ${params.deltaY ?? 480} px`, scrolled);
      }

      case "viewport": {
        const session = requireSession();
        const width = required(params.width, "width");
        const height = required(params.height, "height");
        const viewport = await rpc(
          session,
          "Page.setViewport",
          { width, height },
          params.timeoutMs,
        );
        session.width = width;
        session.height = height;
        return result(`Set viewport to ${width}×${height}`, viewport);
      }

      case "wait": {
        const session = requireSession();
        const state = await rpc(
          session,
          "App.waitForIdle",
          { timeoutMs: params.timeoutMs ?? 30_000 },
          (params.timeoutMs ?? 30_000) + 1_000,
        );
        return result("Kraken is idle", state);
      }

      case "command": {
        const session = requireSession();
        const method = required(params.method, "method");
        const response = await rpc(
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
        return result("Terminated headless Kraken", closed);
      }
    }
  },

  onSession(event) {
    if (event.reason === "shutdown" && active !== undefined) {
      void closeSession(active, 2_000);
    }
  },
});

async function readyEndpoint(
  stream: ReadableStream<Uint8Array>,
  timeoutMs: number,
  signal: AbortSignal,
): Promise<Endpoint> {
  const reader = stream.getReader();
  const decoder = new TextDecoder();
  let buffered = "";
  const deadline = Date.now() + timeoutMs;
  try {
    while (Date.now() < deadline) {
      if (signal.aborted) {
        throw new Error("Kraken launch was cancelled");
      }
      const remaining = Math.max(1, deadline - Date.now());
      const chunk = await withTimeout(reader.read(), remaining, "automation endpoint");
      if (chunk.done) {
        throw new Error("Kraken exited before publishing its automation endpoint");
      }
      buffered += decoder.decode(chunk.value, { stream: true });
      const lines = buffered.split("\n");
      buffered = lines.pop() ?? "";
      for (const line of lines) {
        const payload: unknown = JSON.parse(line);
        if (!isRecord(payload) || payload.method !== "Automation.ready") {
          continue;
        }
        const endpoint = payload.params;
        if (
          !isRecord(endpoint) ||
          typeof endpoint.host !== "string" ||
          typeof endpoint.port !== "number" ||
          typeof endpoint.protocolVersion !== "string"
        ) {
          throw new Error("Kraken published an invalid automation endpoint");
        }
        return {
          host: endpoint.host,
          port: endpoint.port,
          protocolVersion: endpoint.protocolVersion,
        };
      }
    }
    throw new Error(`Kraken did not publish an automation endpoint within ${timeoutMs} ms`);
  } finally {
    reader.releaseLock();
  }
}

async function rpc(
  session: QaSession,
  method: string,
  params: Record<string, unknown>,
  timeoutMs = 15_000,
): Promise<unknown> {
  if (session.child.exitCode !== null) {
    const diagnostic = session.stderr.length > 0 ? `: ${session.stderr.join("\n")}` : "";
    throw new Error(`Headless Kraken has exited${diagnostic}`);
  }
  const id = nextRequestId;
  nextRequestId += 1;

  const { promise, resolve: resolvePromise, reject: rejectPromise } =
    Promise.withResolvers<unknown>();
  const socket = createConnection({ host: session.host, port: session.port });
  let buffered = "";
  let settled = false;
  const finish = (error?: Error, value?: unknown) => {
    if (settled) {
      return;
    }
    settled = true;
    clearTimeout(timer);
    socket.destroy();
    if (error !== undefined) {
      rejectPromise(error);
    } else {
      resolvePromise(value);
    }
  };
  const timer = setTimeout(
    () => finish(new Error(`${method} timed out after ${timeoutMs} ms`)),
    timeoutMs,
  );

  socket.setEncoding("utf8");
  socket.on("connect", () => {
    socket.write(`${JSON.stringify({ id, method, params })}\n`);
  });
  socket.on("data", (chunk) => {
    buffered += chunk;
    const newline = buffered.indexOf("\n");
    if (newline < 0) {
      return;
    }
    let payload: unknown;
    try {
      payload = JSON.parse(buffered.slice(0, newline));
    } catch (error) {
      finish(new Error(`Invalid ${method} response: ${message(error)}`));
      return;
    }
    if (!isRpcResponse(payload) || payload.id !== id) {
      finish(new Error(`Malformed ${method} response`));
      return;
    }
    if (payload.error !== undefined) {
      finish(new Error(payload.error.message));
      return;
    }
    finish(undefined, payload.result);
  });
  socket.on("error", (error) => finish(error));
  socket.on("end", () => {
    if (!settled) {
      finish(new Error(`${method} connection closed without a response`));
    }
  });
  return promise;
}

async function closeSession(session: QaSession, timeoutMs: number): Promise<unknown> {
  let response: unknown;
  try {
    response = await rpc(session, "Browser.close", {}, timeoutMs);
    await withTimeout(session.child.exited, timeoutMs, "Kraken termination");
  } catch (error) {
    if (session.child.exitCode === null) {
      session.child.kill("SIGTERM");
      await withTimeout(session.child.exited, timeoutMs, "forced Kraken termination");
    }
    response = { forced: true, reason: message(error) };
  } finally {
    if (active?.child === session.child) {
      active = undefined;
    }
  }
  return response;
}

async function drain(stream: ReadableStream<Uint8Array>, lines: string[]): Promise<void> {
  const reader = stream.getReader();
  const decoder = new TextDecoder();
  let buffered = "";
  while (true) {
    const chunk = await reader.read();
    if (chunk.done) {
      break;
    }
    buffered += decoder.decode(chunk.value, { stream: true });
    const complete = buffered.split("\n");
    buffered = complete.pop() ?? "";
    lines.push(...complete.filter(Boolean));
    if (lines.length > 100) {
      lines.splice(0, lines.length - 100);
    }
  }
  if (buffered.length > 0) {
    lines.push(buffered);
  }
}

async function withTimeout<T>(
  promise: Promise<T>,
  timeoutMs: number,
  label: string,
): Promise<T> {
  const timeout = Promise.withResolvers<T>();
  const timer = setTimeout(
    () => timeout.reject(new Error(`${label} timed out`)),
    timeoutMs,
  );
  try {
    return await Promise.race([promise, timeout.promise]);
  } finally {
    clearTimeout(timer);
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

function result(text: string, details: unknown) {
  return {
    content: [{ type: "text", text }],
    details: { details },
  };
}

function sessionDetails(session: QaSession) {
  return {
    pid: session.child.pid,
    host: session.host,
    port: session.port,
    width: session.width,
    height: session.height,
  };
}

function snapshotText(snapshot: unknown): string {
  if (!isRecord(snapshot)) {
    return JSON.stringify(snapshot, null, 2);
  }
  const text = Array.isArray(snapshot.text)
    ? snapshot.text
        .filter(isRecord)
        .map((item) => item.text)
        .filter((item): item is string => typeof item === "string")
    : [];
  const hits = Array.isArray(snapshot.hits)
    ? snapshot.hits
        .filter(isRecord)
        .map((item) => item.action)
        .filter((item): item is string => typeof item === "string")
    : [];
  return [
    `Visible text (${text.length}):`,
    ...text,
    "",
    `Hit targets (${hits.length}):`,
    ...hits,
  ].join("\n");
}

function targetName(value: unknown): string {
  if (isRecord(value) && typeof value.target === "string") {
    return value.target;
  }
  return "coordinates";
}

function isRpcResponse(value: unknown): value is RpcResponse {
  if (!isRecord(value) || typeof value.id !== "number") {
    return false;
  }
  if (value.error === undefined) {
    return true;
  }
  return isRecord(value.error) && typeof value.error.message === "string";
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export default factory;
