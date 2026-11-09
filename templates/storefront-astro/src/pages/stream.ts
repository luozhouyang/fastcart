export const prerender = false;

const encoder = new TextEncoder();

export function GET() {
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
}
