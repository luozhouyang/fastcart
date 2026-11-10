import { value } from "./util.js";

export default {
  async fetch() {
    return Response.json({ value });
  },
};
