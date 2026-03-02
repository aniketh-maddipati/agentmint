#!/usr/bin/env python3
"""
AgentMint Asciinema Demo
========================
30-second highlight reel. No interaction, just plays.

Record with:
    asciinema rec demo.cast -c "python3 asciinema_demo.py"

Requires: AgentMint server on localhost:3000
"""

import requests
import time
import sys

BASE = "http://localhost:3000"

# Colors (true color)
RESET = "\033[0m"
BOLD = "\033[1m"
DIM = "\033[2m"
GREEN = "\033[38;2;34;197;94m"    # #22c55e
YELLOW = "\033[38;2;250;204;21m"  # amber
RED = "\033[38;2;239;68;68m"      # red-500
CYAN = "\033[38;2;34;211;238m"    # cyan-400
GRAY = "\033[38;2;107;114;128m"   # gray-500
WHITE = "\033[38;2;255;255;255m"
CLEAR = "\033[2J\033[H"


def clear():
    print(CLEAR, end="", flush=True)


def title_card(text, subtext=""):
    """Centered title card — gives viewer a beat to reset."""
    clear()
    print("\n" * 8)
    print(f"{WHITE}{BOLD}{text:^70}{RESET}")
    if subtext:
        print(f"{GRAY}{subtext:^70}{RESET}")
    print("\n" * 8)
    time.sleep(2.5)


def banner(text, color):
    """Compact colored banner."""
    print(f"\n  {color}{BOLD}{'━' * 3} {text} {'━' * 3}{RESET}\n")


def check(agent, action, note=""):
    """Green checkmark line."""
    print(f"  {GREEN}✓{RESET}  {WHITE}{agent}{RESET} → {CYAN}{action}{RESET}  {GRAY}{note}{RESET}")


def checkpoint_line(agent, action):
    """Yellow checkpoint line."""
    print(f"  {YELLOW}⏸{RESET}  {WHITE}{agent}{RESET} → {YELLOW}{action}{RESET}")
    print(f"      {YELLOW}checkpoint_required{RESET}")


def denied(agent, action, reason=""):
    """Red denied line."""
    print(f"  {RED}✗{RESET}  {WHITE}{agent}{RESET} → {RED}{action}{RESET}")
    if reason:
        print(f"      {RED}{reason}{RESET}")


def center(text, width=60):
    """Center text in terminal."""
    return f"{text:^{width}}"


def show_plan(plan):
    """Show the plan being approved — centered."""
    lines = [
        ("action", plan["action"]),
        ("scope", ", ".join(plan["scope"])),
        ("delegates_to", ", ".join(plan["delegates_to"])),
        ("requires_checkpoint", ", ".join(plan["requires_checkpoint"]) or "none"),
    ]
    
    max_key = max(len(k) for k, v in lines)
    
    for key, val in lines:
        line = f"{GRAY}{key}:{RESET} {WHITE}{val}{RESET}"
        padding = " " * 20
        print(f"{padding}{GRAY}{key:>{max_key}}:{RESET} {WHITE}{val}{RESET}")


def check_server():
    try:
        return requests.get(f"{BASE}/health", timeout=2).status_code == 200
    except:
        return False


# ════════════════════════════════════════════════════════════
#  MOMENTS
# ════════════════════════════════════════════════════════════

def moment_1():
    """PLAN — Human approves, receipt minted."""
    title_card("1 · PLAN", "human approves a scoped action")
    
    clear()
    print()
    print(f"{GRAY}{'POST /mint':^70}{RESET}")
    print(f"{GRAY}{'─' * 50:^70}{RESET}")
    print()
    time.sleep(0.8)
    
    plan = {
        "sub": "aniketh@company.com",
        "action": "deploy:api-v2",
        "ttl_seconds": 300,
        "scope": ["build:*", "test:*", "deploy:staging"],
        "delegates_to": ["build-agent", "test-agent", "deploy-agent"],
        "requires_checkpoint": ["deploy:production"],
        "max_delegation_depth": 2,
    }
    
    show_plan(plan)
    
    time.sleep(2)
    
    r = requests.post(f"{BASE}/mint", json=plan)
    resp = r.json()
    
    print()
    print(f"{' ' * 22}{GREEN}✓{RESET} {GRAY}minted:{RESET} {WHITE}{resp['jti'][:8]}...{RESET}")
    
    time.sleep(1.5)
    
    print()
    print(f"{GREEN}{BOLD}{'━━━ PLAN APPROVED ━━━':^70}{RESET}")
    print()
    time.sleep(3)


def moment_2():
    """DELEGATE — Fast approvals, then checkpoint."""
    title_card("2 · DELEGATE", "agents request scoped actions")
    
    clear()
    print()
    print(f"{GRAY}{'POST /delegate':^70}{RESET}")
    print(f"{GRAY}{'─' * 50:^70}{RESET}")
    print()
    time.sleep(1)
    
    # Get a fresh token
    plan = {
        "sub": "aniketh@company.com",
        "action": "deploy:api-v2",
        "ttl_seconds": 300,
        "scope": ["build:*", "test:*", "deploy:staging"],
        "delegates_to": ["build-agent", "test-agent", "deploy-agent"],
        "requires_checkpoint": ["deploy:production"],
        "max_delegation_depth": 2,
    }
    token = requests.post(f"{BASE}/mint", json=plan).json()["token"]
    
    # Three fast approvals
    delegates = [
        ("build-agent", "build:docker", "build:*"),
        ("test-agent", "test:integration", "test:*"),
        ("deploy-agent", "deploy:staging", "deploy:staging"),
    ]
    
    pad = " " * 18
    for agent, action, scope in delegates:
        requests.post(f"{BASE}/delegate", json={
            "parent_token": token,
            "agent_id": agent,
            "action": action,
        })
        print(f"{pad}{GREEN}✓{RESET}  {WHITE}{agent:14}{RESET} → {CYAN}{action:18}{RESET} {GRAY}{scope}{RESET}")
        time.sleep(1.5)
    
    # The checkpoint — this is the key moment
    print()
    time.sleep(1)
    
    requests.post(f"{BASE}/delegate", json={
        "parent_token": token,
        "agent_id": "deploy-agent",
        "action": "deploy:production",
    })
    
    print(f"{pad}{YELLOW}⏸{RESET}  {WHITE}{'deploy-agent':14}{RESET} → {YELLOW}{'deploy:production':18}{RESET}")
    print(f"{pad}   {YELLOW}checkpoint_required{RESET}")
    
    time.sleep(1.5)
    print()
    print(f"{YELLOW}{BOLD}{'━━━ CHECKPOINT ━━━':^70}{RESET}")
    print(f"{GRAY}{'production requires human approval':^70}{RESET}")
    time.sleep(4)


def moment_3():
    """DENY — Unauthorized agent blocked, audit trail."""
    title_card("3 · DENY", "unauthorized agent blocked")
    
    clear()
    print()
    print(f"{GRAY}{'POST /delegate':^70}{RESET}")
    print(f"{GRAY}{'─' * 50:^70}{RESET}")
    print()
    time.sleep(1)
    
    # Fresh token
    plan = {
        "sub": "aniketh@company.com",
        "action": "deploy:api-v2",
        "ttl_seconds": 300,
        "scope": ["build:*"],
        "delegates_to": ["build-agent"],
        "requires_checkpoint": [],
        "max_delegation_depth": 2,
    }
    token = requests.post(f"{BASE}/mint", json=plan).json()["token"]
    
    resp = requests.post(f"{BASE}/delegate", json={
        "parent_token": token,
        "agent_id": "rogue-agent",
        "action": "build:docker",
    }).json()
    
    pad = " " * 18
    print(f"{pad}{RED}✗{RESET}  {WHITE}{'rogue-agent':14}{RESET} → {RED}{'build:docker':18}{RESET}")
    print(f"{pad}   {RED}{resp.get('reason', 'agent_not_authorized')}{RESET}")
    
    time.sleep(1.5)
    print()
    print(f"{RED}{BOLD}{'━━━ DENIED ━━━':^70}{RESET}")
    time.sleep(3)
    
    # Quick audit
    print()
    print(f"{GRAY}{'GET /audit':^70}{RESET}")
    print()
    time.sleep(0.8)
    
    entries = requests.get(f"{BASE}/audit").json()[:3]
    
    print(f"{' ' * 10}{GRAY}{'─' * 56}{RESET}")
    for e in entries:
        ts = e.get('verified_at', '')[:19]
        sub = e.get('sub', 'unknown')[:15]
        action = e.get('action', 'unknown')[:22]
        print(f"{' ' * 10}{GRAY}{ts}{RESET}  {WHITE}{sub:15}{RESET}  {CYAN}{action:22}{RESET}")
        time.sleep(0.5)
    print(f"{' ' * 10}{GRAY}{'─' * 56}{RESET}")
    
    time.sleep(3)


def end_card():
    """Final card."""
    clear()
    print("\n" * 8)
    print(f"{GREEN}{BOLD}{'AgentMint':^70}{RESET}")
    print()
    print(f"{WHITE}{'cryptographic receipts for AI agent delegation':^70}{RESET}")
    print()
    print(f"{CYAN}{'github.com/aniketh-maddipati/agentmint':^70}{RESET}")
    print("\n" * 8)
    time.sleep(4)


# ════════════════════════════════════════════════════════════
#  MAIN
# ════════════════════════════════════════════════════════════

def main():
    if not check_server():
        print(f"{RED}Server not running at {BASE}{RESET}")
        print(f"{GRAY}Start with: cd ~/agentmint && cargo run{RESET}")
        sys.exit(1)
    
    moment_1()   # ~10s
    moment_2()   # ~16s
    moment_3()   # ~14s
    end_card()   # ~4s
    # Total: ~44s


if __name__ == "__main__":
    main()