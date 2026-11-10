export default {
  async fetch() {
    await new Promise((resolve) => setTimeout(resolve, 1500));
    return new Response("slow");
  },
};
