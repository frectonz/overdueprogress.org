use std::path::PathBuf;

use clap::Parser;
use color_eyre::eyre::{Context, Result, eyre};
use serde::Deserialize;
use sqlx::SqlitePool;

#[derive(Parser)]
#[command(about = "Export the reviewer's shortlist (top 10 + close calls) to CSV for the judges")]
struct Cli {
    /// SQLite connection string
    #[arg(long, env = "DATABASE_URL", default_value = "sqlite://submissions.db")]
    database_url: String,

    /// JSON picks file with `top` and `close` arrays of submission ids
    /// (kept out of git).
    #[arg(long, default_value = "shortlist.json")]
    picks: PathBuf,

    /// Output CSV path
    #[arg(short, long, default_value = "shortlist.csv")]
    out: PathBuf,
}

#[derive(Deserialize)]
struct Picks {
    top: Vec<i64>,
    close: Vec<i64>,
}

#[derive(sqlx::FromRow)]
struct Essay {
    id: i64,
    title: String,
    author: String,
    link: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    let raw = std::fs::read_to_string(&cli.picks)
        .with_context(|| format!("reading picks file {}", cli.picks.display()))?;
    let picks: Picks = serde_json::from_str(&raw).context("parsing picks JSON")?;

    let db = SqlitePool::connect(&cli.database_url).await?;
    let rows = sqlx::query_as::<_, Essay>("SELECT id, title, author, link FROM submissions")
        .fetch_all(&db)
        .await?;
    let by_id: std::collections::HashMap<i64, &Essay> = rows.iter().map(|e| (e.id, e)).collect();

    let mut csv = String::from('\u{feff}');
    let header: Vec<&str> = ["List", "Rank", "Title", "Author", "Link"]
        .into_iter()
        .chain(SCORE_COLUMNS.iter().copied())
        .collect();
    push_row(&mut csv, &header);
    for (rank, id) in picks.top.iter().enumerate() {
        let essay = by_id
            .get(id)
            .ok_or_else(|| eyre!("missing submission id {id}"))?;
        write_row(&mut csv, "Top 10", &(rank + 1).to_string(), essay);
    }
    for id in &picks.close {
        let essay = by_id
            .get(id)
            .ok_or_else(|| eyre!("missing submission id {id}"))?;
        write_row(&mut csv, "Close call", "", essay);
    }

    std::fs::write(&cli.out, csv)?;
    println!(
        "Wrote {} ({} top + {} close calls).",
        cli.out.display(),
        picks.top.len(),
        picks.close.len()
    );
    Ok(())
}

const SCORE_COLUMNS: &[&str] = &[
    "Quality of research (out of 10)",
    "Originality of thinking (out of 10)",
    "Clarity of writing (out of 10)",
    "Specificity of analysis (out of 10)",
    "Rigor of analysis (out of 10)",
    "Comments",
];

fn write_row(csv: &mut String, list: &str, rank: &str, essay: &Essay) {
    let mut fields = vec![list, rank, &essay.title, &essay.author, &essay.link];
    fields.extend(std::iter::repeat_n("", SCORE_COLUMNS.len()));
    push_row(csv, &fields);
}

fn push_row(csv: &mut String, fields: &[&str]) {
    let line: Vec<String> = fields.iter().map(|f| csv_field(f)).collect();
    csv.push_str(&line.join(","));
    csv.push_str("\r\n");
}

fn csv_field(s: &str) -> String {
    if s.contains(['"', ',', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}
