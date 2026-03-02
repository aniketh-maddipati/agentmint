#!/usr/bin/env python3
"""
AgentMint Orchestration Demo
=============================
Demonstrates multi-agent delegation with scoped authorization,
checkpoint escalation, and denial for unauthorized agents.

Requires: AgentMint server running on localhost:3000
    cd ~/agentmint && cargo run

Usage:
    python3 demo.py
"""

import requests
import json
import time
import sys

BASE = "http://localhost:3000"

# --- Colors ---
RESET = "\033[0m"
BOLD = "\033[1m"
DIM = "\033[2m"
GREEN = "\033[32m"
RED = "\033[31m"
YELLOW = "\033[33m"
CYAN = "\033[36m"
WHITE = "\033[37m"
BG_GREEN = "\033[42m"
BG_RED = "\033[41m"
BG_YELLOW = "\033[43m"
BG_BLUE = "\033[44m"
BG_CYAN = "\033[46m"
BLACK = "\033[30m"


def badge(text, fg, bg):
    return f"{fg}{bg}{BOLD} {text} {RESET}"


def step(n, title):
    print()
    print(f"{DIM}{'─' * 60}{RESET}")
    print(f"{BOLD}{WHITE}  Step {n}: {title}{RESET}")
    print(f"{DIM}{'─' * 60}{RESET}")
    print()


def agent_line(agent, action, status, detail=""):
    icons = {"ok": f"{GREEN}✓{RESET}", "denied": f"{RED}✗{RESET}", "checkpoint": f"{YELLOW}⚠{RESET}"}
    icon = icons.get(status, "?")
    print(f"  {icon} {BOLD}{agent}{RESET} → {CYAN}{action}{RESET}  {detail}")


def show_chain(chain):
    print(f"\n  {DIM}Chain:{RESET}")
    for i, jti in enumerate(chain):
        prefix = "  └─" if i == len(chain) - 1 else "  ├─"
        print(f"  {DIM}{prefix}{RESET} {jti[:8]}...")


def pause(seconds=1.5):
    time.sleep(seconds)


def check_server():
    try:
        r = requests.get(f"{BASE}/health", timeout=2)
        return r.status_code == 200
    except Exception:
        return False


# ============================================================
#  MAIN DEMO
# ============================================================

def main():
    print()
    print(f"{BOLD}{CYAN}╔══════════════════════════════════════════════════════╗{RESET}")
    print(f"{BOLD}{CYAN}║{RESET}  {BOLD}AgentMint Orchestration Demo{RESET}                        {BOLD}{CYAN}║{RESET}")
    print(f"{BOLD}{CYAN}║{RESET}  {DIM}Scoped delegation · Checkpoint escalation · Denial{RESET}  {BOLD}{CYAN}║{RESET}")
    print(f"{BOLD}{CYAN}╚══════════════════════════════════════════════════════╝{RESET}")
    print()

    if not check_server():
        print(f"{RED}  AgentMint server not running. Start it:{RESET}")
        print(f"{DIM}    cd ~/agentmint && cargo run{RESET}")
        sys.exit(1)

    print(f"  {GREEN}✓{RESET} AgentMint server connected at {CYAN}{BASE}{RESET}")

    # ── Step 1: Human approves a plan ──────────────────────

    step(1, "Human approves deployment plan")

    plan = {
        "sub": "aniketh@company.com",
        "action": "deploy:api-v2",
        "ttl_seconds": 300,
        "scope": ["build:*", "test:*", "deploy:staging"],
        "delegates_to": ["build-agent", "test-agent", "deploy-agent"],
        "requires_checkpoint": ["deploy:production"],
        "max_delegation_depth": 2,
    }

    print(f"  {DIM}Plan:{RESET}")
    print(f"    Scope:       {CYAN}{', '.join(plan['scope'])}{RESET}")
    print(f"    Agents:      {CYAN}{', '.join(plan['delegates_to'])}{RESET}")
    print(f"    Checkpoints: {YELLOW}{', '.join(plan['requires_checkpoint'])}{RESET}")
    print()

    input(f"  {BOLD}Press Enter to approve this plan...{RESET}")

    r = requests.post(f"{BASE}/mint", json=plan)
    resp = r.json()
    token = resp["token"]

    print()
    print(f"  {badge('PLAN', BLACK, BG_GREEN)} Receipt minted")
    print(f"    JTI:  {DIM}{resp['jti'][:8]}...{RESET}")
    print(f"    Type: {GREEN}plan{RESET}")
    print(f"    Exp:  {DIM}{resp['exp']}{RESET}")

    pause()

    # ── Step 2: Build agent requests delegation ────────────

    step(2, "Build agent: build:docker")

    r = requests.post(f"{BASE}/delegate", json={
        "parent_token": token,
        "agent_id": "build-agent",
        "action": "build:docker",
    })
    resp = r.json()

    agent_line("build-agent", "build:docker", resp["status"],
               f"{DIM}matches scope build:*{RESET}")
    if resp.get("chain"):
        show_chain(resp["chain"])

    pause()

    # ── Step 3: Test agent requests delegation ─────────────

    step(3, "Test agent: test:integration")

    r = requests.post(f"{BASE}/delegate", json={
        "parent_token": token,
        "agent_id": "test-agent",
        "action": "test:integration",
    })
    resp = r.json()

    agent_line("test-agent", "test:integration", resp["status"],
               f"{DIM}matches scope test:*{RESET}")
    if resp.get("chain"):
        show_chain(resp["chain"])

    pause()

    # ── Step 4: Deploy agent - staging (in scope) ──────────

    step(4, "Deploy agent: deploy:staging")

    r = requests.post(f"{BASE}/delegate", json={
        "parent_token": token,
        "agent_id": "deploy-agent",
        "action": "deploy:staging",
    })
    resp = r.json()

    agent_line("deploy-agent", "deploy:staging", resp["status"],
               f"{DIM}explicit scope match{RESET}")
    if resp.get("chain"):
        show_chain(resp["chain"])

    pause()

    # ── Step 5: Deploy agent - production (CHECKPOINT) ─────

    step(5, "Deploy agent: deploy:production")

    r = requests.post(f"{BASE}/delegate", json={
        "parent_token": token,
        "agent_id": "deploy-agent",
        "action": "deploy:production",
    })
    resp = r.json()

    agent_line("deploy-agent", "deploy:production", "checkpoint",
               f"{YELLOW}{resp.get('reason', '')}{RESET}")

    print()
    print(f"  {badge('CHECKPOINT', BLACK, BG_YELLOW)} Agent cannot proceed without explicit approval")
    print()

    input(f"  {BOLD}Approve production deploy? Press Enter to approve...{RESET}")

    # Human explicitly approves production deploy
    prod_plan = {
        "sub": "aniketh@company.com",
        "action": "deploy:production",
        "ttl_seconds": 60,
        "scope": ["deploy:production"],
        "delegates_to": ["deploy-agent"],
        "requires_checkpoint": [],
        "max_delegation_depth": 1,
    }

    r = requests.post(f"{BASE}/mint", json=prod_plan)
    prod_resp = r.json()
    prod_token = prod_resp["token"]

    print()
    print(f"  {badge('APPROVED', BLACK, BG_GREEN)} New receipt minted for deploy:production")
    print(f"    JTI: {DIM}{prod_resp['jti'][:8]}...{RESET}")

    pause(0.5)

    # Now deploy agent can proceed with new token
    r = requests.post(f"{BASE}/delegate", json={
        "parent_token": prod_token,
        "agent_id": "deploy-agent",
        "action": "deploy:production",
    })
    resp = r.json()

    agent_line("deploy-agent", "deploy:production", resp["status"],
               f"{GREEN}explicitly approved{RESET}")
    if resp.get("chain"):
        show_chain(resp["chain"])

    pause()

    # ── Step 6: Rogue agent (DENIED) ───────────────────────

    step(6, "Rogue agent: unauthorized access attempt")

    r = requests.post(f"{BASE}/delegate", json={
        "parent_token": token,
        "agent_id": "rogue-agent",
        "action": "build:docker",
    })
    resp = r.json()

    agent_line("rogue-agent", "build:docker", resp["status"],
               f"{RED}{resp.get('reason', '')}{RESET}")

    pause()

    # ── Step 7: Audit trail ────────────────────────────────

    step(7, "Audit trail")

    r = requests.get(f"{BASE}/audit")
    entries = r.json()

    print(f"  {BOLD}Last {min(len(entries), 6)} receipts:{RESET}")
    print()
    for entry in entries[:6]:
        print(f"    {DIM}{entry['verified_at'][:19]}{RESET}  "
              f"{BOLD}{entry['sub']:15}{RESET}  "
              f"{CYAN}{entry['action']:25}{RESET}  "
              f"{DIM}{entry['jti'][:8]}...{RESET}")

    # ── Done ───────────────────────────────────────────────

    print()
    print(f"{DIM}{'─' * 60}{RESET}")
    print()
    print(f"  {BOLD}{GREEN}Demo complete.{RESET}")
    print()
    print(f"  {DIM}Every action above was cryptographically signed with Ed25519.{RESET}")
    print(f"  {DIM}Every receipt is single-use (JTI tracked), time-limited (TTL),{RESET}")
    print(f"  {DIM}and chains back to the original human approval.{RESET}")
    print()
    print(f"  {DIM}Code: https://github.com/aniketh-maddipati/agentmint{RESET}")
    print()


if __name__ == "__main__":
    main()