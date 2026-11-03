import { createServer } from "node:http";
import { Readable } from "node:stream";

import handler from "../dist/server/server.js";

const port = Number(process.env.PORT ?? 3000);
const host = process.env.HOST ?? "0.0.0.0";

function toHeaders(incomingHeaders) {
  const headers = new Headers();
  for (const [name, value] of Object.entries(incomingHeaders)) {
    if (Array.isArray(value)) {
      for (const item of value) {
        headers.append(name, item);
      }
    } else if (value !== undefined) {
      headers.set(name, value);
    }
  }
  return headers;
}

const server = createServer(async (request, response) => {
  try {
    const url = new URL(request.url ?? "/", `http://${request.headers.host ?? "localhost"}`);
    const hasBody = request.method !== "GET" && request.method !== "HEAD";
    const webRequest = new Request(url, {
      method: request.method,
      headers: toHeaders(request.headers),
      body: hasBody ? Readable.toWeb(request) : undefined,
      duplex: hasBody ? "half" : undefined,
    });

    const webResponse = await handler.fetch(webRequest);

    response.statusCode = webResponse.status;
    for (const [name, value] of webResponse.headers) {
      if (name === "set-cookie") {
        response.setHeader(name, webResponse.headers.getSetCookie());
      } else {
        response.setHeader(name, value);
      }
    }

    if (webResponse.body) {
      Readable.fromWeb(webResponse.body).pipe(response);
    } else {
      response.end();
    }
  } catch (error) {
    console.error(error);
    if (!response.headersSent) {
      response.statusCode = 500;
    }
    response.end("Internal Server Error");
  }
});

server.listen(port, host, () => {
  console.log(`storefront listening on http://${host}:${port}`);
});
