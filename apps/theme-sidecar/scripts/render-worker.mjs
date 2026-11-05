// Isolated render worker.
//
// Each request imports the theme entry and calls its render export inside a
// short-lived worker thread. The main sidecar terminates the worker when the
// request deadline passes, so a runaway theme cannot block other renders or
// the health endpoint. Resource limits cap the heap the theme can allocate.
//
// A theme may return a string, bytes, or an async iterable / web stream. When
// the host asked for streaming, the worker forwards chunks one at a time and
// waits for an ack before pulling the next one, so the sidecar never buffers a
// whole page on behalf of a slow client.

import { parentPort, workerData } from "node:worker_threads";
import { pathToFileURL } from "node:url";

const { entryFile, version, payload } = workerData;

const MAX_RESPONSE_BYTES = 10 * 1024 * 1024;
const MAX_CHUNK_BYTES = 1024 * 1024;

let ackResolver = null;
parentPort.on("message", (message) => {
  if (message?.type === "ack" && ackResolver) {
    const resolve = ackResolver;
    ackResolver = null;
    resolve();
  }
});

async function main() {
  const url = `${pathToFileURL(entryFile).href}?v=${encodeURIComponent(String(version))}`;
  const module = await import(url);
  const render = renderFunction(module);
  if (!render) {
    throw new Error("theme entry does not export a render function");
  }
  const result = await render(payload);
  if (!isRecord(result)) {
    throw new Error("theme render did not return an object");
  }

  const body = result.body;
  if (!isStreamingBody(body)) {
    parentPort.postMessage({ ok: true, result });
    return;
  }

  if (payload?.streaming !== true) {
    // The host does not accept raw streams. Buffer the result and answer with
    // the legacy JSON shape instead of failing the render.
    const bytes = await collectBody(body);
    parentPort.postMessage({
      ok: true,
      result: {
        status: result.status,
        headers: result.headers ?? [],
        body: bytes,
      },
    });
    return;
  }

  await streamBody(result, body);
}

async function streamBody(result, body) {
  parentPort.postMessage({
    ok: true,
    stream: true,
    status: result.status,
    headers: result.headers ?? [],
  });

  let total = 0;
  for await (const value of bodyChunks(body)) {
    const chunk = toBuffer(value);
    if (!chunk) {
      throw new Error("theme stream produced an invalid chunk");
    }
    if (chunk.length > MAX_CHUNK_BYTES) {
      throw new Error("theme stream chunk exceeded 1 MiB");
    }
    total += chunk.length;
    if (total > MAX_RESPONSE_BYTES) {
      throw new Error("theme response exceeded 10 MiB");
    }
    await sendChunk(chunk);
  }
  parentPort.postMessage({ ok: true, stream: true, done: true });
}

function sendChunk(chunk) {
  return new Promise((resolve) => {
    ackResolver = resolve;
    parentPort.postMessage({ ok: true, stream: true, chunk });
  });
}

async function collectBody(body) {
  const chunks = [];
  let total = 0;
  for await (const value of bodyChunks(body)) {
    const chunk = toBuffer(value);
    if (!chunk) {
      throw new Error("theme stream produced an invalid chunk");
    }
    total += chunk.length;
    if (total > MAX_RESPONSE_BYTES) {
      throw new Error("theme response exceeded 10 MiB");
    }
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}

async function* bodyChunks(body) {
  if (typeof body[Symbol.asyncIterator] === "function") {
    yield* body;
    return;
  }
  if (typeof body.getReader === "function") {
    const reader = body.getReader();
    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) {
          break;
        }
        yield value;
      }
    } finally {
      reader.releaseLock();
    }
    return;
  }
  throw new Error("theme returned an unsupported stream");
}

function isStreamingBody(body) {
  if (body === null || body === undefined || typeof body === "string") {
    return false;
  }
  if (body instanceof Uint8Array) {
    return false;
  }
  return typeof body[Symbol.asyncIterator] === "function" || typeof body.getReader === "function";
}

function toBuffer(value) {
  if (typeof value === "string") {
    return Buffer.from(value, "utf8");
  }
  if (value instanceof Uint8Array) {
    return Buffer.from(value.buffer, value.byteOffset, value.byteLength);
  }
  if (value instanceof ArrayBuffer) {
    return Buffer.from(value);
  }
  return null;
}

function renderFunction(module) {
  if (typeof module?.render === "function") {
    return module.render;
  }
  if (typeof module?.default?.render === "function") {
    return module.default.render;
  }
  if (typeof module?.default === "function") {
    return module.default;
  }
  return null;
}

function isRecord(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

main().catch((error) => {
  parentPort.postMessage({
    ok: false,
    message: error instanceof Error ? error.message : String(error),
  });
});
