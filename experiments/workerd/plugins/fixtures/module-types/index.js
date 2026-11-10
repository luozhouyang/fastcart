import data from "./data.json";
import note from "./note.txt";

export default {
  async fetch() {
    return Response.json({ data, note });
  },
};
