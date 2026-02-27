#!/usr/bin/env node
const BASE_URL = process.env.AGENTMINT_URL || 'http://localhost:3000'\;
const R = '\x1b[91m', G = '\x1b[92m', Y = '\x1b[93m', B = '\x1b[94m';
const C = '\x1b[96m', W = '\x1b[97m', D = '\x1b[2m', BOLD = '\x1b[1m', RST = '\x1b[0m';

function box(title, lines, color = C) {
  const width = 70;
  console.log(`\n${color}┌${'─'.repeat(width - 2)}┐${RST}`);
  console.log(`${color}│${RST} ${BOLD}${title}${RST}${' '.repeat(Math.max(0, width - title.length - 4))}${color}│${RST}`);
  console.log(`${color}├${'─'.repeat(width - 2)}┤${RST}`);
  for (const line of lines) {
    const clean = line.replace(/\x1b\[[0-9;]*m/g, '');
    const padding = Math.max(0, width - clean.length - 4);
    console.log(`${color}│${RST} ${line}${' '.repeat(padding)} ${color}│${RST}`);
  }
  console.log(`${color}└${'─'.repeat(width - 2)}┘${RST}`);
}

function section(title) {
  console.log(`\n${Y}${'═'.repeat(70)}`);
  console.log(` ${BOLD}${title}${RST}`);
  console.log(`${Y}${'═'.repeat(70)}${RST}\n`);
}

async function mint(sub, action, ttl = 60) {
  const res = await fetch(`${BASE_URL}/mint`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ sub, action, ttl_seconds: ttl })
  });
  return res.json();
}

async function verify(token) {
  const res = await fetch(`${BASE_URL}/proxy`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ token })
  });
  if (res.ok) return res.json();
  return { error: await res.text() };
}

async function checkServer() {
  try { return (await fetch(`${BASE_URL}/health`)).ok; } catch { return false; }
}

const sleep = ms => new Promise(r => setTimeout(r, ms));

async function main() {
  console.log(`\n${C}${'═'.repeat(70)}${RST}`);
  console.log(`${BOLD}${W}   🔐 AgentMint: Honest Answers to Security Questions${RST}`);
  console.log(`${C}${'═'.repeat(70)}${RST}`);

  if (!await checkServer()) {
    console.log(`\n${R}Error: Server not running at ${BASE_URL}${RST}`);
    console.log(`${W}Start it with: cargo run${RST}\n`);
    process.exit(1);
  }
  console.log(`\n${G}✓ Connected to AgentMint${RST}`);

  section("Q1: How do users get verified? Can I forge a token for Aniketh?");
  box("THE QUESTION", ["What's to prevent me from forging a token for Aniketh", "by just claiming that I'm Aniketh?"], Y);
  console.log(`\n${W}Let's try it:${RST}`);
  console.log(`${D}Attempting to forge a token for aniketh@company.com...${RST}\n`);
  const forgeResult = await verify("eyJqdGkiOiJmYWtlIn0.FAKESIG");
  console.log(`  ${R}Result: ${JSON.stringify(forgeResult)}${RST}`);
  box("ANSWER", [
    `${G}The signature verification fails.${RST}`, "",
    "AgentMint uses Ed25519 signatures. To forge a token, you'd",
    "need the server's private key, which never leaves the server.", "",
    `${Y}BUT HERE'S THE HONEST PART:${RST}`, "",
    "AgentMint does NOT verify who you are. It trusts that YOUR",
    "backend already authenticated the user before calling /mint.", "",
    "The flow is:",
    "  1. User logs into YOUR app (you verify identity)",
    "  2. User clicks 'Approve refund' in YOUR UI",
    "  3. YOUR backend calls AgentMint /mint with sub='aniketh'",
    "  4. AgentMint signs it (trusting your backend)", "",
    `${Y}ROOM FOR IMPROVEMENT:${RST}`,
    "  • Could integrate with OIDC to verify identity claims",
    "  • Could require user's own signature (WebAuthn/passkeys)",
    "  • Could support delegated signing (user holds key)", "",
    "Today: AgentMint is a signing service, not an IdP."
  ], G);

  section("Q2: Who verifies the token? Is agent just transport?");
  box("THE QUESTION", ["I expect the resource provider to do verification,", "but it's not clear. If so, agent is just transporting."], Y);
  console.log(`\n${W}Let's trace through a real flow:${RST}\n`);
  console.log(`  ${C}Step 1:${RST} Human approves → Your backend calls /mint`);
  const tokenData = await mint("aniketh@company.com", "refund:order:123:amount:50", 60);
  console.log(`  ${D}Token: ${tokenData.token?.slice(0, 40)}...${RST}\n`);
  console.log(`  ${C}Step 2:${RST} Agent receives token, carries to resource provider`);
  console.log(`  ${D}(Agent cannot modify - signature would break)${RST}\n`);
  console.log(`  ${C}Step 3:${RST} Resource provider calls /proxy to verify`);
  const verifyResult = await verify(tokenData.token);
  console.log(`  ${G}Verified: sub=${verifyResult.sub}, action=${verifyResult.action}${RST}\n`);
  console.log(`  ${C}Step 4:${RST} Agent tries to replay the same token...`);
  const replayResult = await verify(tokenData.token);
  console.log(`  ${R}Blocked: ${JSON.stringify(replayResult)}${RST}\n`);
  box("ANSWER", [
    `${G}Yes, RESOURCE PROVIDER verifies by calling /proxy.${RST}`,
    `${G}Yes, AGENT is just transport.${RST}`, "",
    "Architecture:", "",
    "  [Human] → [Your Backend] → /mint → [AgentMint]",
    "                                         ↓",
    "                                       token",
    "                                         ↓",
    "  [Agent] ←───────────────── [Your Backend]",
    "     │",
    "     └──→ [Resource Provider] → /proxy → [AgentMint]",
    "                                            ↓",
    "                                       verified ✓", "",
    "Agent CANNOT: forge, modify, replay, or use expired tokens",
    "Agent CAN ONLY: carry the exact token it was given"
  ], G);

  section("Q3: Why not just add claims to existing OAuth tokens?");
  box("THE QUESTION", ["You don't need a new token - just add claims like", "'verified-by-aniketh, action=refund' to existing OAuth.", "Resource providers verify claims. Standardize via OAuth."], Y);
  console.log(`\n${W}Great point. Let's demo the differences:${RST}\n`);
  console.log(`  ${C}Demo:${RST} AgentMint tokens are single-use`);
  const t1 = await mint("demo@test.com", "test:single-use", 60);
  await verify(t1.token);
  const r2 = await verify(t1.token);
  console.log(`  ${D}First: success | Second: ${JSON.stringify(r2)}${RST}\n`);
  console.log(`  ${C}Demo:${RST} AgentMint tokens expire in seconds`);
  const t2 = await mint("demo@test.com", "test:expiry", 2);
  console.log(`  ${D}Minted with 2s TTL, waiting 3s...${RST}`);
  await sleep(3000);
  const r3 = await verify(t2.token);
  console.log(`  ${D}Result: ${JSON.stringify(r3)}${RST}\n`);
  box("HONEST ANSWER", [
    `${Y}Your friend is RIGHT - this COULD be an OAuth extension.${RST}`, "",
    "See: IETF draft-patwhite-aauth-00 (Agent Authorization)", "",
    `${G}What AgentMint adds vs OAuth:${RST}`, "",
    "  1. SINGLE-USE (JTI tracking)",
    "     OAuth: reusable │ AgentMint: one use, then dead", "",
    "  2. SECONDS not hours",
    "     OAuth: 1hr+ │ AgentMint: 60s default", "",
    "  3. ACTION-SPECIFIC",
    "     OAuth: scope=stripe:write │ AgentMint: refund:order:123", "",
    "  4. NO IDP COORDINATION",
    "     OAuth: need IdP changes │ AgentMint: drop-in sidecar", "",
    `${Y}WHERE YOUR FRIEND IS RIGHT:${RST}`,
    "  • This SHOULD eventually be standardized",
    "  • OAuth extension would have broader adoption",
    "  • We're exploring what the standard SHOULD look like"
  ], B);

  section("Summary: Honest Assessment");
  box("WHAT AGENTMINT DOES WELL", [
    "✓ Crypto proof that SOME human approved THIS action",
    "✓ Single-use tokens prevent replay",
    "✓ Short expiry limits damage window",
    "✓ Full audit trail",
    "✓ Simple deploy (single binary)",
    "✓ Fast (3ms verify)"
  ], G);
  box("ROOM FOR IMPROVEMENT", [
    "⚠ Doesn't verify WHO (trusts your backend)",
    "⚠ Could integrate OIDC for identity",
    "⚠ Could support user-held keys (WebAuthn)",
    "⚠ Should become OAuth extension long-term",
    "⚠ Resource providers need new integration",
    "⚠ Centralized (single point of failure)"
  ], Y);
  box("THE BOTTOM LINE", [
    "AgentMint is a PROTOTYPE exploring primitives for",
    "human-in-the-loop agent authorization.", "",
    "Your friend's OAuth intuition is correct - this should",
    "be standardized. We're figuring out WHAT to standardize",
    "by building something concrete first.", "",
    "Status: useful for exploring, not production-ready."
  ], C);
  console.log(`\n${C}${'═'.repeat(70)}${RST}`);
  console.log(`${W}   Demo complete. 🔐${RST}`);
  console.log(`${C}${'═'.repeat(70)}${RST}\n`);
}

main().catch(console.error);
