import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/__fastcart/health")({
  server: {
    handlers: {
      GET: () => Response.json({ status: "ok", framework: "tanstack-start" }),
    },
  },
});
