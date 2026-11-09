import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/redirect")({
  server: {
    handlers: {
      GET: () =>
        new Response(null, {
          status: 302,
          headers: { location: "/" },
        }),
    },
  },
});
