//! Pretty terminal output with colors and badges.

use colored::Colorize;

// === Startup ===

pub fn print_banner() {
    println!();
    println!("{}", "╔═══════════════════════════════════════════════════════════╗".cyan());
    println!("{}", "║                                                           ║".cyan());
    println!("║     {}     ║", "🔐 AgentMint v0.1.0".bold().white());
    println!("║     {}     ║", "Cryptographic proof of human authorization".dimmed());
    println!("{}", "║                                                           ║".cyan());
    println!("{}", "╚═══════════════════════════════════════════════════════════╝".cyan());
    println!();
}

pub fn print_startup(addr: &str) {
    println!("{} {}", "✓".green().bold(), "Server ready".white().bold());
    println!("  {} {}", "→".dimmed(), format!("http://{}", addr).cyan().underline());
    println!();
    println!("{}", "Endpoints:".white().bold());
    println!("  {} {}  {}", "POST".yellow(), "/mint".white(), "Issue signed token".dimmed());
    println!("  {} {}  {}", "POST".yellow(), "/proxy".white(), "Verify & consume token".dimmed());
    println!("  {} {}  {}", "GET ".green(), "/audit".white(), "View audit log".dimmed());
    println!("  {} {} {}", "GET ".green(), "/metrics".white(), "Telemetry".dimmed());
    println!("  {} {} {}", "GET ".green(), "/health".white(), "Health check".dimmed());
    println!();
    println!("{}", "WebAuthn:".white().bold());
    println!("  {} {} {}", "POST".yellow(), "/webauthn/register/start".white(), "Begin registration".dimmed());
    println!("  {} {} {}", "POST".yellow(), "/webauthn/register/finish".white(), "Complete registration".dimmed());
    println!("  {} {} {}", "POST".yellow(), "/webauthn/auth/start".white(), "Begin authentication".dimmed());
    println!("  {} {} {}", "POST".yellow(), "/webauthn/auth/finish".white(), "Complete authentication".dimmed());
    println!();
}

// === Badges ===

fn badge(text: &str, fg: colored::Color, bg: colored::Color) -> colored::ColoredString {
    format!(" {} ", text).color(fg).on_color(bg).bold()
}

fn short_jti(jti: &str) -> &str {
    if jti.len() >= 8 { &jti[..8] } else { jti }
}

// === Core Events ===

pub fn log_mint(sub: &str, action: &str, jti: &str) {
    println!(
        "{} {} {} {} {} {}",
        badge("MINT", colored::Color::Black, colored::Color::Green),
        "sub:".dimmed(), sub.white(),
        "action:".dimmed(), action.cyan(),
        format!("jti:{}", short_jti(jti)).dimmed()
    );
}

pub fn log_verify(jti: &str, time_us: u128) {
    println!(
        "{} {} {} {}",
        badge("OK", colored::Color::Black, colored::Color::Blue),
        format!("jti:{}", short_jti(jti)).white(),
        format!("{}μs", time_us).green(),
        "✓".green().bold()
    );
}

pub fn log_reject(reason: &str) {
    println!("{} {}", badge("DENY", colored::Color::White, colored::Color::Red), reason.red());
}

pub fn log_replay(jti: &str) {
    println!(
        "{} {} {}",
        badge("REPLAY", colored::Color::Black, colored::Color::Yellow),
        format!("jti:{}", short_jti(jti)).white(),
        "blocked".yellow()
    );
}

// === Policy ===

pub fn log_policy_denial(sub: &str, action: &str, action_type: &str, limit: u64, requested: u64) {
    println!(
        "{} {} {} {} {} {} {}",
        badge("POLICY", colored::Color::White, colored::Color::Red),
        format!("sub:{}", sub).yellow(),
        format!("type:{}", action_type).cyan(),
        format!("action:{}", action).white(),
        format!("limit=${}", limit).dimmed(),
        format!("req=${}", requested).red(),
        "DENIED".red().bold()
    );
}

// === OIDC ===

pub fn log_oidc_success(sub: &str) {
    println!(
        "{} {} {} {}",
        badge("OIDC", colored::Color::Black, colored::Color::Green),
        "sub:".dimmed(), sub.white(),
        "✓ verified".green()
    );
}

pub fn log_oidc_failure(sub: &str, reason: &str) {
    println!(
        "{} {} {} {}",
        badge("OIDC", colored::Color::White, colored::Color::Red),
        format!("sub:{}", sub).yellow(),
        "failed:".dimmed(),
        reason.red()
    );
}

pub fn log_oidc_mismatch(requested: &str, actual: &str) {
    println!(
        "{} {} {} {}",
        badge("OIDC", colored::Color::White, colored::Color::Red),
        "mismatch:".dimmed(),
        format!("requested={}", requested).yellow(),
        format!("actual={}", actual).red()
    );
}

pub fn log_oidc_required(sub: &str) {
    println!(
        "{} {} {}",
        badge("OIDC", colored::Color::Black, colored::Color::Yellow),
        format!("sub:{}", sub).yellow(),
        "id_token required".yellow()
    );
}

// === Rate Limiting ===

pub fn log_rate_limited(ip: &str, reason: &str) {
    println!(
        "{} {} {} {}",
        badge("RATE", colored::Color::Black, colored::Color::Yellow),
        format!("ip:{}", ip).yellow(),
        "→".dimmed(),
        reason.yellow()
    );
}

// === WebAuthn ===

pub fn log_webauthn_register(user_id: &str) {
    println!(
        "{} {} {} {}",
        badge("WEBAUTHN", colored::Color::Black, colored::Color::Green),
        "user:".dimmed(), user_id.white(),
        "✓ registered".green()
    );
}

pub fn log_webauthn_auth(user_id: &str) {
    println!(
        "{} {} {} {}",
        badge("WEBAUTHN", colored::Color::Black, colored::Color::Blue),
        "user:".dimmed(), user_id.white(),
        "✓ authenticated".green()
    );
}

pub fn log_webauthn_failure(user_id: &str) {
    println!(
        "{} {} {} {}",
        badge("WEBAUTHN", colored::Color::White, colored::Color::Red),
        "user:".dimmed(), user_id.yellow(),
        "✗ auth failed".red()
    );
}

pub fn log_webauthn_lockout(user_id: &str) {
    println!(
        "{} {} {} {}",
        badge("LOCKOUT", colored::Color::White, colored::Color::Red),
        "user:".dimmed(), user_id.yellow(),
        "🔒 account locked".red()
    );
}
