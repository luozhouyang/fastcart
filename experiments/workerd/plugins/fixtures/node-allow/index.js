import { Buffer } from "node:buffer";
import process from "node:process";
import path from "node:path";
import util from "node:util";
import crypto from "node:crypto";
import { Readable } from "node:stream";
import { EventEmitter } from "node:events";

export default {
  async fetch() {
    return Response.json({
      buffer: Buffer.from("hi").toString("base64"),
      processType: typeof process,
      envKeys: process?.env ? Object.keys(process.env).sort() : null,
      joined: path.join("a", "b"),
      utilFormat: typeof util.format,
      uuidLength: crypto.randomUUID().length,
      readable: typeof Readable,
      events: typeof EventEmitter,
    });
  },
};
