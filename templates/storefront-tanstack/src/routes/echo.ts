import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/echo")({
  server: {
    handlers: {
      POST: async ({ request }) =>
        Response.json({
          method: request.method,
          store: request.headers.get("x-fastcart-store-id"),
          deployment: request.headers.get("x-fastcart-deployment-id"),
          requestId: request.headers.get("x-fastcart-request-id"),
          proto: request.headers.get("x-forwarded-proto"),
          body: await request.text(),
        }),
    },
  },
});
