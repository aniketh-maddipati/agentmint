//! Pretty console output.

use colored::Colorize;

pub fn print_banner() {
    println!();
    println!("{}", "╔═══════════════════════════════════════════════════════════╗".cyan());
    println!("{}", "║                                                           ║".cyan());
    println!("║  {}  ║", "   🔐 AgentMint v0.1.0".bold().white());
    println!("║  {}  ║", "   Cryptographic proof of human authorization".dimmed());
    println!("{}", "║                                                           ║".cyan());
    println!("{}", "╚═══════════════════════════════════════════════════════════╝".cyan());
    println!();
}

pub fn print_startup(addr: &str) {
    println!("{} {}", "✓".green().bold(), "Server started".white());
    println!("  {} {}", "→".dimmed(), format!("http://{}", addr).cyan().underline());
    println!();
    println!("{}", "Endpoints:".white().bold());
    println!("  {} {} {}", "POST".yellow(), "/mint ".white(), "Issue signed token".dimmed());
    println!("  {} {} {}", "POST".yellow(), "/proxy".white(), "Verify token".dimmed());
    println!("  {} {} {}", "GET ".green(), "/audit".white(), "View audit log".dimmed());
    println!("  {} {} {}", "GET ".green(), "/metrics".white(), "Telemetry".dimmed());
    println!("  {} {} {}", "GET ".green(), "/health".white(), "Health check".dimmed());
    println!();
}

pub fn log_mint(sub: &str, action: &str, jti: &str) {
    let short_jti = if jti.len() >= 8 { &jti[..8] } else { jti };
    println!(
        "{} {} {} {} {} {}",
        " MINT ".black().on_green().bold(),
        "sub:".dimmed(),
        sub.white(),
        "action:".dimmed(),
        action.cyan(),
        format!("jti:{}", short_jti).dimmed()
    );
}

pub fn log_verify(jti: &str, time_us: u128) {
    let short_jti = if jti.len() >= 8 { &jti[..8] } else { jti };
    println!(
        "{} {} {} {}",
        " OK ".black().on_blue().bold(),
        format!("jti:{}", short_jti).white(),
        format!("{}μs", time_us).green(),
        "✓".green().bold()
    );
}

pub fn log_reject(reason: &str) {
    println!("{} {}", " DENY ".black().on_red().bold(), reason.red());
}

pub fn log_replay(jti: &str) {
    let short_jti = if jti.len() >= 8 { &jti[..8] } else { jti };
    println!(
        "{} {} {}",
        " REPLAY ".black().on_yellow().bold(),
        format!("jti:{}", short_jti).white(),
        "blocked".yellow()
    );
}

pub fn log_policy_denial(sub: &str, action: &str, action_type: &str, limit: u64, requested: u64) {
    println!(
        "{} {} {} {} {} {} {} {}",
        " POLICY ".black().on_red().bold(),
        "sub:".dimmed(),
        sub.yellow(),
        "action:".dimmed(),
        action.cyan(),
        format!("limit=${}", limit).dimmed(),
        format!("requested=${}", requested).red(),
        "DENIED".red().bold()
    );
}