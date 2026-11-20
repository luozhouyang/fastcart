export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const count = Number(url.searchParams.get("n") ?? 8);
    const statuses = [];
    for (let index = 0; index < count; index += 1) {
      try {
        const response = await env.FASTCART_API.fetch("http://bridge/internal/plugins/v1/context");
        statuses.push(response.status);
      } catch (error) {
        statuses.push(`error:${String(error).slice(0, 40)}`);
      }
    }
    return Response.json({ statuses });
  },
};
