import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/boom")({
  component: Boom,
});

function Boom(): never {
  throw new Error("intentional fixture error");
}
