// Phase 0 streaming Worker: each chunk must reach the client before the next one.
const CHUNKS = 3;
const DELAY_MS = 300;

export default {
  async fetch() {
    const encoder = new TextEncoder();
    let index = 0;
    const stream = new ReadableStream({
      async pull(controller) {
        if (index >= CHUNKS) {
          controller.close();
          return;
        }
        controller.enqueue(encoder.encode(`chunk-${index}\n`));
        index += 1;
        await new Promise((resolve) => setTimeout(resolve, DELAY_MS));
      },
    });
    return new Response(stream, {
      headers: {
        "content-type": "text/plain; charset=utf-8",
        "cache-control": "no-store",
      },
    });
  },
};
