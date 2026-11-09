import { createFileRoute } from "@tanstack/react-router";
import { createServerFn } from "@tanstack/react-start";

const loadMessage = createServerFn().handler(async () => ({
  message: "FastCart TanStack SSR",
}));

export const Route = createFileRoute("/")({
  loader: () => loadMessage(),
  component: Home,
});

function Home() {
  const data = Route.useLoaderData();
  return (
    <main>
      <h1>{data.message}</h1>
      <p data-testid="hydration">Server and client share one route tree.</p>
    </main>
  );
}
