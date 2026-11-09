import { createFileRoute } from "@tanstack/react-router";

const encoder = new TextEncoder();

export const Route = createFileRoute("/stream")({
  server: {
    handlers: {
      GET: () => {
        const stream = new ReadableStream({
          async start(controller) {
            for (const chunk of ["shell-", "streamed-", "done"]) {
              controller.enqueue(encoder.encode(chunk));
              await new Promise((resolve) => setTimeout(resolve, 100));
            }
            controller.close();
          },
        });
        return new Response(stream, {
          headers: { "content-type": "text/plain; charset=utf-8" },
        });
      },
    },
  },
});
