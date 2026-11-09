const encoder = new TextEncoder();

function html(body, assetsPrefix) {
  return new Response(
    `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>FastCart Generic Storefront</title></head><body>${body}<script type="module" src="${assetsPrefix}/app.js"></script></body></html>`,
    { headers: { "content-type": "text/html; charset=utf-8" } },
  );
}

export default {
  async fetch(request) {
    const url = new URL(request.url);
    if (url.pathname === "/__fastcart/health") {
      return Response.json({ status: "ok", framework: "generic" });
    }
    if (url.pathname === "/echo") {
      return Response.json({
        method: request.method,
        store: request.headers.get("x-fastcart-store-id"),
        deployment: request.headers.get("x-fastcart-deployment-id"),
        requestId: request.headers.get("x-fastcart-request-id"),
        proto: request.headers.get("x-forwarded-proto"),
        preview: request.headers.get("x-fastcart-preview"),
        body: await request.text(),
      });
    }
    if (url.pathname === "/stream") {
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
    if (url.pathname === "/cookie") {
      const headers = new Headers();
      headers.append("set-cookie", "a=1; Path=/; HttpOnly");
      headers.append("set-cookie", "b=2; Path=/; HttpOnly");
      return new Response("cookies", { headers });
    }
    if (url.pathname === "/redirect") {
      return new Response(null, {
        status: 302,
        headers: { location: "/" },
      });
    }
    if (url.pathname === "/boom") {
      throw new Error("intentional fixture error");
    }
    const assetsPrefix = request.headers.get("x-fastcart-assets-prefix") ?? "/_fastcart/assets";
    return html(
      "<main><h1>FastCart Generic Fetch Handler</h1><p>No framework adapter.</p></main>",
      assetsPrefix,
    );
  },
};
