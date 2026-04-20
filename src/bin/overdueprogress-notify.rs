use std::process::Command;

use clap::{Parser, Subcommand};
use color_eyre::eyre::Result;
use html_escape::encode_text;
use overdueprogress::telegram::Client;

#[derive(Parser)]
#[command(about = "Send ops alerts to Telegram")]
struct Cli {
    #[arg(long, env = "BOT_TOKEN", hide_env_values = true)]
    bot_token: String,

    #[arg(long, env = "CHAT_ID")]
    chat_id: String,

    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Notify about a failed systemd unit
    Failure {
        /// The unit name (e.g. overdueprogress.service)
        unit: String,
    },
    /// Notify on host boot
    Boot,
    /// Scan for systemd units in failed state and notify if any exist
    FailedUnits,
    /// Notify on SSH session open (invoked from PAM with PAM_* env vars set)
    SshLogin,
    /// Check disk usage; notify only when the level (ok/warn/critical) changes
    DiskCheck {
        /// Mount path to check
        #[arg(long, default_value = "/")]
        path: String,
        /// Percent-used threshold for warn level
        #[arg(long, default_value_t = 85)]
        warn: u64,
        /// Percent-used threshold for critical level
        #[arg(long, default_value_t = 95)]
        critical: u64,
        /// Directory for the persisted level state file
        #[arg(long, default_value = "/var/lib/telegram-notify")]
        state_dir: String,
    },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let client = Client::new(http, cli.bot_token, cli.chat_id);

    let text = match cli.command {
        Cmd::Failure { unit } => failure_message(&unit),
        Cmd::Boot => boot_message(),
        Cmd::FailedUnits => match failed_units_message() {
            Some(t) => t,
            None => return Ok(()),
        },
        Cmd::SshLogin => match ssh_login_message() {
            Some(t) => t,
            None => return Ok(()),
        },
        Cmd::DiskCheck {
            path,
            warn,
            critical,
            state_dir,
        } => match disk_check_message(&path, warn, critical, &state_dir)? {
            Some(t) => t,
            None => return Ok(()),
        },
    };

    client.send(text).await?;
    Ok(())
}

fn failure_message(unit: &str) -> String {
    let status = run(&[
        "systemctl",
        "show",
        unit,
        "-p",
        "ActiveState,SubState,Result,ExecMainStatus",
        "--value",
    ])
    .unwrap_or_else(|_| "unknown".into())
    .lines()
    .collect::<Vec<_>>()
    .join(" | ");

    let tail = run(&[
        "journalctl",
        "-u",
        unit,
        "-n",
        "20",
        "--no-pager",
        "-o",
        "short-iso",
    ])
    .unwrap_or_else(|_| "(no log output)".into());
    let tail = safe_tail(&tail, 3000);

    format!(
        "⚠️ <b>{}</b> failed on <code>{}</code>\nstatus: <code>{}</code>\n\n<pre>{}</pre>",
        encode_text(unit),
        encode_text(&host()),
        encode_text(&status),
        encode_text(tail),
    )
}

fn boot_message() -> String {
    let booted = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    format!(
        "🟢 <b>{}</b> booted\nkernel: <code>{}</code>\ntime: <code>{}</code>",
        encode_text(&host()),
        encode_text(&kernel()),
        encode_text(&booted),
    )
}

fn ssh_login_message() -> Option<String> {
    if std::env::var("PAM_TYPE").ok()? != "open_session" {
        return None;
    }
    let user = std::env::var("PAM_USER").unwrap_or_else(|_| "unknown".into());
    let rhost = std::env::var("PAM_RHOST").unwrap_or_else(|_| "local".into());
    let tty = std::env::var("PAM_TTY").unwrap_or_else(|_| "unknown".into());
    Some(format!(
        "🔐 ssh login on <code>{}</code>\nuser: <code>{}</code>\nfrom: <code>{}</code>\ntty: <code>{}</code>",
        encode_text(&host()),
        encode_text(&user),
        encode_text(&rhost),
        encode_text(&tty),
    ))
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum DiskLevel {
    Ok,
    Warn,
    Critical,
}

impl DiskLevel {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Critical => "critical",
        }
    }
    fn parse(s: &str) -> Self {
        match s.trim() {
            "warn" => Self::Warn,
            "critical" => Self::Critical,
            _ => Self::Ok,
        }
    }
    fn classify(used: u64, warn: u64, critical: u64) -> Self {
        if used >= critical {
            Self::Critical
        } else if used >= warn {
            Self::Warn
        } else {
            Self::Ok
        }
    }
    fn emoji(&self) -> &'static str {
        match self {
            Self::Ok => "🟢",
            Self::Warn => "🟡",
            Self::Critical => "🔴",
        }
    }
}

fn disk_check_message(
    path: &str,
    warn: u64,
    critical: u64,
    state_dir: &str,
) -> Result<Option<String>> {
    let used = disk_used_percent(path)?;
    let current = DiskLevel::classify(used, warn, critical);

    std::fs::create_dir_all(state_dir)?;
    let state_file = std::path::Path::new(state_dir).join("disk-state");
    let last = std::fs::read_to_string(&state_file)
        .map(|s| DiskLevel::parse(&s))
        .unwrap_or(DiskLevel::Ok);

    std::fs::write(&state_file, current.as_str())?;

    if current == last {
        return Ok(None);
    }

    Ok(Some(format!(
        "{} disk on <code>{}</code>: <b>{}%</b> used at <code>{}</code> (was {}, now {})",
        current.emoji(),
        encode_text(&host()),
        used,
        encode_text(path),
        last.as_str(),
        current.as_str(),
    )))
}

fn disk_used_percent(path: &str) -> Result<u64> {
    let out = Command::new("df").args(["-P", path]).output()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let row = stdout
        .lines()
        .last()
        .ok_or_else(|| color_eyre::eyre::eyre!("empty df output"))?;
    let pct = row
        .split_whitespace()
        .nth(4)
        .ok_or_else(|| color_eyre::eyre::eyre!("unexpected df format: {row}"))?
        .trim_end_matches('%');
    Ok(pct.parse()?)
}

fn failed_units_message() -> Option<String> {
    let out = run(&["systemctl", "--failed", "--no-legend", "--plain"]).ok()?;
    let units: Vec<&str> = out
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    if units.is_empty() {
        return None;
    }
    let list = units.join("\n");
    Some(format!(
        "🔴 failed units on <code>{}</code>\n\n<pre>{}</pre>",
        encode_text(&host()),
        encode_text(&list),
    ))
}

fn run(argv: &[&str]) -> std::io::Result<String> {
    let out = Command::new(argv[0]).args(&argv[1..]).output()?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn read_trim(path: &str) -> String {
    std::fs::read_to_string(path)
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

fn host() -> String {
    read_trim("/proc/sys/kernel/hostname")
}

fn kernel() -> String {
    read_trim("/proc/sys/kernel/osrelease")
}

fn safe_tail(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut i = s.len() - max_bytes;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    &s[i..]
}
