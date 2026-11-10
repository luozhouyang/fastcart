/**
 * Deterministic, dependency-free tar.gz writer for plugin artifacts.
 *
 * Entries must be supplied in sorted order with forward-slash paths. Timestamps
 * are fixed at zero so identical inputs produce identical archives.
 */

import { gzipSync } from "node:zlib";

export interface TarEntry {
  path: string;
  data: Buffer;
  mode?: number;
}

const BLOCK = 512;
const EMPTY = Buffer.alloc(BLOCK);

/** Creates an uncompressed ustar archive. */
export function createTar(entries: readonly TarEntry[]): Buffer {
  const blocks: Buffer[] = [];
  for (const entry of entries) {
    blocks.push(header(entry.path, entry.data.length, entry.mode ?? 0o644));
    blocks.push(entry.data);
    const padding = (BLOCK - (entry.data.length % BLOCK)) % BLOCK;
    if (padding > 0) blocks.push(Buffer.alloc(padding));
  }
  blocks.push(EMPTY, EMPTY);
  return Buffer.concat(blocks);
}

/** Creates a deterministic gzip-compressed ustar archive. */
export function createTarGz(entries: readonly TarEntry[]): Buffer {
  return gzipSync(createTar(entries), { level: 9 });
}

function header(path: string, size: number, mode: number): Buffer {
  const header = Buffer.alloc(BLOCK);
  const { name, prefix } = splitPath(path);
  header.write(name, 0, 100, "utf8");
  writeOctal(header, 100, 8, mode & 0o7777);
  writeOctal(header, 108, 8, 0);
  writeOctal(header, 116, 8, 0);
  writeOctal(header, 124, 12, size);
  writeOctal(header, 136, 12, 0);
  header.write("        ", 148, 8, "ascii");
  header[156] = 0x30; // regular file
  header.write("ustar\0", 257, 6, "ascii");
  header.write("00", 263, 2, "ascii");
  header.write("root", 265, 32, "utf8");
  header.write("root", 297, 32, "utf8");
  if (prefix.length > 0) header.write(prefix, 345, 155, "utf8");
  let checksum = 0;
  for (const byte of header) checksum += byte;
  header.write(`${checksum.toString(8).padStart(6, "0")}\0 `, 148, 8, "ascii");
  return header;
}

function splitPath(path: string): { name: string; prefix: string } {
  const encoded = Buffer.from(path, "utf8");
  if (encoded.length <= 100) return { name: path, prefix: "" };
  // Split at a slash so both fields fit.
  const parts = path.split("/");
  for (let index = parts.length - 1; index > 0; index -= 1) {
    const prefix = parts.slice(0, index).join("/");
    const name = parts.slice(index).join("/");
    if (Buffer.byteLength(prefix) <= 155 && Buffer.byteLength(name) <= 100) {
      return { name, prefix };
    }
  }
  throw new Error(`tar path is too long: ${path}`);
}

function writeOctal(buffer: Buffer, offset: number, length: number, value: number): void {
  const text = value.toString(8).padStart(length - 1, "0");
  buffer.write(`${text}\0`, offset, length, "ascii");
}
