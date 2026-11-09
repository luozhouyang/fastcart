import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/cookie")({
  server: {
    handlers: {
      GET: () => {
        const headers = new Headers();
        headers.append("set-cookie", "a=1; Path=/; HttpOnly");
        headers.append("set-cookie", "b=2; Path=/; HttpOnly");
        return new Response("cookies", { headers });
      },
    },
  },
});
