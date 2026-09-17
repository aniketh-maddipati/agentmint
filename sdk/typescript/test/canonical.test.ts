import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

function canonicalize(value: unknown): string {
  if (value === null) return "null";
  if (typeof value === "boolean") return value ? "true" : "false";
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new Error("non-finite number");
    return JSON.stringify(value);
  }
  if (typeof value === "string") return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(canonicalize).join(",")}]`;
  if (typeof value === "object") {
    const keys = Object.keys(value as Record<string, unknown>).sort();
    return `{${keys
      .map((key) => `${JSON.stringify(key)}:${canonicalize((value as Record<string, unknown>)[key])}`)
      .join(",")}}`;
  }
  throw new Error("unsupported value");
}

function hashMaterial(material: unknown): string {
  const canonical = canonicalize(material);
  return `sha256:${createHash("sha256").update(canonical).digest("hex")}`;
}

const root = join(dirname(fileURLToPath(import.meta.url)), "../../..");

test("rust and typescript share canonical hashes for fixtures", () => {
  const base = JSON.parse(
    readFileSync(join(root, "fixtures/canonicalization/support-refund-base.json"), "utf8"),
  );
  const reordered = JSON.parse(
    readFileSync(join(root, "fixtures/canonicalization/support-refund-key-reorder.json"), "utf8"),
  );
  const left = hashMaterial(base.material);
  const right = hashMaterial(reordered.material);
  assert.equal(left, right);
  assert.equal(left, base.expected_sha256);
  assert.equal(right, reordered.expected_sha256);
});
