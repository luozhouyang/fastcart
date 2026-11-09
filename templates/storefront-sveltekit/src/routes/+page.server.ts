import type { PageServerLoad } from "./$types";

export const load: PageServerLoad = ({ request }) => ({
  message: "FastCart SvelteKit SSR",
  store: request.headers.get("x-fastcart-store-id"),
  deployment: request.headers.get("x-fastcart-deployment-id"),
});
