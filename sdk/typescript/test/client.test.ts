import assert from "node:assert/strict";
import test from "node:test";
import { Mint, encodeDevToken } from "../src/index.ts";

test("client encodes local identity and builds action URLs", async () => {
  const token = encodeDevToken({
    tenantId: "acme",
    subject: "user_123",
    agentId: "support-agent-7",
    issuer: "https://identity.example.com",
  });
  assert.match(token, /^dev\./);

  const calls: Array<{ url: string; method: string; body: unknown }> = [];
  const mint = new Mint({
    baseUrl: "http://127.0.0.1:8787",
    token,
    fetch: async (input, init) => {
      const url = String(input);
      const method = init?.method ?? "GET";
      const body = init?.body ? JSON.parse(String(init.body)) : undefined;
      calls.push({ url, method, body });
      return new Response(
        JSON.stringify({
          id: "00000000-0000-0000-0000-000000000001",
          status: "Authorized",
          tenantId: "acme",
          intentHash: "sha256:abc",
          provider: "fake",
          operation: "refund.create",
          resource: { type: "charge", id: "ch_123" },
          arguments: body?.arguments ?? { amount: 4200 },
          context: { supportTicketId: "ticket_982" },
          actor: { subject: "user_123", agent_id: "support-agent-7", issuer: "https://identity.example.com" },
          expiresAt: new Date().toISOString(),
          reconciliationRequired: false,
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      );
    },
  });

  await mint.actions.propose({
    tenantId: "acme",
    actor: {
      subject: "user_123",
      agentId: "support-agent-7",
      issuer: "https://identity.example.com",
    },
    provider: "stripe",
    operation: "refund.create",
    resource: { type: "charge", id: "ch_123" },
    arguments: { amount: 4200, currency: "usd", reason: "duplicate" },
    context: { supportTicketId: "ticket_982" },
  });
  await mint.actions.execute("00000000-0000-0000-0000-000000000001");
  assert.equal(calls[0]?.url, "http://127.0.0.1:8787/v1/actions");
  assert.equal(calls[1]?.url, "http://127.0.0.1:8787/v1/actions/00000000-0000-0000-0000-000000000001/execute");
  assert.equal(calls[1]?.body && Object.keys(calls[1].body as object).length, 0);
});
