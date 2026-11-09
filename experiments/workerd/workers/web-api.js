// Phase 0 Web API probe: exercises the APIs the universal SSR runtime needs.
function utf8Hex(buffer) {
  return [...new Uint8Array(buffer)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

export default {
  async fetch() {
    const encoder = new TextEncoder();
    const decoder = new TextDecoder();
    const digest = await crypto.subtle.digest("SHA-256", encoder.encode("fastcart"));

    const source = new ReadableStream({
      start(controller) {
        controller.enqueue(encoder.encode("stream-"));
        controller.enqueue(encoder.encode("works"));
        controller.close();
      },
    });
    const reader = source.getReader();
    let streamed = "";
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      streamed += decoder.decode(value, { stream: true });
    }
    streamed += decoder.decode();

    const headers = new Headers({ "x-fastcart-test": "yes" });
    headers.append("x-fastcart-test", "again");
    const request = new Request("https://shop.example/products/example?q=1", {
      headers,
      method: "POST",
      body: "payload",
    });

    const timer = await new Promise((resolve) => {
      const started = Date.now();
      setTimeout(() => resolve(Date.now() - started), 20);
    });

    const controller = new AbortController();
    let abortObserved = false;
    controller.signal.addEventListener("abort", () => {
      abortObserved = true;
    });
    controller.abort();

    return Response.json({
      apis: {
        request: typeof Request === "function",
        response: typeof Response === "function",
        headers: typeof Headers === "function",
        url: typeof URL === "function",
        readableStream: typeof ReadableStream === "function",
        writableStream: typeof WritableStream === "function",
        textEncoder: typeof TextEncoder === "function",
        textDecoder: typeof TextDecoder === "function",
        crypto: typeof crypto?.subtle === "object",
        abortController: typeof AbortController === "function",
        timers: typeof setTimeout === "function",
        fetch: typeof fetch === "function",
      },
      request: {
        method: request.method,
        path: new URL(request.url).pathname,
        header: headers.get("x-fastcart-test"),
        body: await request.text(),
      },
      streamed,
      sha256: utf8Hex(digest),
      timerElapsedMs: timer,
      abortObserved,
    });
  },
};
