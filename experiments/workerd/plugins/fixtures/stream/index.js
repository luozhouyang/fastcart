const encoder = new TextEncoder();

export default {
  async fetch() {
    const stream = new ReadableStream({
      async start(controller) {
        controller.enqueue(encoder.encode("first"));
        await new Promise((resolve) => setTimeout(resolve, 300));
        controller.enqueue(encoder.encode("second"));
        controller.close();
      },
    });
    return new Response(stream, { headers: { "content-type": "text/plain" } });
  },
};
