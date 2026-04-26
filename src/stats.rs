use std::collections::HashMap;

use axum::{Router, extract::State, response::Response, routing::get};
use axum_extra::extract::cookie::CookieJar;
use minijinja::context;
use serde::Serialize;
use time::{Duration as TimeDuration, OffsetDateTime, format_description::well_known::Iso8601};
use url::Url;

use crate::AppState;
use crate::auth;
use crate::error::AppError;
use crate::submissions::DEADLINE;

pub fn routes() -> Router<AppState> {
    Router::new().route("/admin/stats", get(stats_page))
}

const TOP_TITLE_WORDS: usize = 40;
const TOP_DESC_WORDS: usize = 60;
const TOP_DOMAINS: usize = 8;
const MIN_WORD_LEN: usize = 3;

const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "of", "in", "on", "at", "to", "from", "by", "for",
    "with", "as", "is", "are", "was", "were", "be", "been", "being", "am", "has", "have", "had",
    "having", "do", "does", "did", "doing", "done", "will", "would", "could", "should", "may",
    "might", "must", "can", "shall", "that", "this", "these", "those", "it", "its", "they", "them",
    "their", "theirs", "we", "our", "us", "ours", "you", "your", "yours", "i", "my", "me", "mine",
    "he", "she", "him", "her", "his", "hers", "not", "no", "nor", "so", "if", "then", "than",
    "too", "also", "just", "very", "quite", "really", "more", "most", "much", "many", "some",
    "any", "all", "each", "every", "only", "even", "still", "yet", "here", "there", "where",
    "when", "why", "how", "what", "who", "whom", "which", "whose", "into", "onto", "out", "off",
    "up", "down", "over", "under", "again", "ever", "never", "always", "often", "about", "against",
    "between", "through", "during", "before", "after", "above", "below", "without", "within",
    "along", "across", "behind", "beyond", "upon", "such", "while", "because", "though",
    "although", "yes", "okay", "well", "like", "get", "got", "make", "made", "one", "two", "way",
    "thing", "things", "lot", "lots", "don", "didn", "doesn", "won", "wouldn", "couldn", "shouldn",
    "isn", "aren", "wasn", "weren", "haven", "hasn", "hadn", "ll", "ve", "re",
];

#[derive(Serialize, Default)]
struct StatsContext {
    total: usize,
    distinct_authors: usize,
    distinct_email_domains: usize,
    distinct_link_domains: usize,
    last_24h: usize,
    final_24h_before_deadline: usize,
    last_submission_at: Option<String>,
    desc_word_avg: usize,
    desc_word_median: usize,
    desc_word_max: usize,
    desc_word_min: usize,
    title_word_avg: usize,
    edits_total: i64,
    reverts_total: i64,
    submissions_edited: i64,
    duplicate_authors_count: usize,
    title_words: Vec<WordFreq>,
    description_words: Vec<WordFreq>,
    timeline: Vec<DayCount>,
    hour_of_day: Vec<HourCount>,
    email_domains: Vec<DomainCount>,
    link_domains: Vec<DomainCount>,
    duplicate_authors: Vec<DuplicateAuthor>,
}

#[derive(Serialize)]
struct WordFreq {
    word: String,
    count: usize,
    weight: f64,
}

#[derive(Serialize)]
struct DayCount {
    day: String,
    label: String,
    count: usize,
    weight: f64,
}

#[derive(Serialize)]
struct HourCount {
    hour: u8,
    label: String,
    count: usize,
    weight: f64,
}

#[derive(Serialize)]
struct DomainCount {
    domain: String,
    count: usize,
    weight: f64,
}

#[derive(Serialize)]
struct DuplicateAuthor {
    name: String,
    count: usize,
    emails: Vec<String>,
}

struct AuthorAgg {
    display: String,
    emails: Vec<String>,
    count: usize,
}

struct Row {
    title: String,
    description: String,
    author: String,
    email: String,
    link: String,
    created_at: String,
}

async fn stats_page(State(state): State<AppState>, jar: CookieJar) -> Result<Response, AppError> {
    if let Some(redirect) = auth::require_session(&state, &jar).await {
        tracing::debug!("stats page accessed without session; redirecting");
        return Ok(redirect);
    }

    let rows = sqlx::query_as!(
        Row,
        "SELECT title, description, author, email, link, created_at
         FROM submissions ORDER BY id ASC",
    )
    .fetch_all(&state.db)
    .await?;

    let edits = sqlx::query!(
        r#"SELECT
            COUNT(*) AS "total!: i64",
            COALESCE(SUM(CASE WHEN edit_kind = 'revert' THEN 1 ELSE 0 END), 0) AS "reverts!: i64",
            COUNT(DISTINCT submission_id) AS "edited_subs!: i64"
           FROM submission_edits"#
    )
    .fetch_one(&state.db)
    .await?;

    let stats = compute_stats(&rows, edits.total, edits.reverts, edits.edited_subs);

    tracing::info!(total = stats.total, "stats page rendered");
    Ok(state.view.render("stats.html", context! { stats => stats }))
}

fn compute_stats(
    rows: &[Row],
    edits_total: i64,
    reverts_total: i64,
    submissions_edited: i64,
) -> StatsContext {
    let mut stats = StatsContext {
        edits_total,
        reverts_total,
        submissions_edited,
        ..Default::default()
    };
    stats.total = rows.len();
    if rows.is_empty() {
        return stats;
    }

    let now = OffsetDateTime::now_utc();
    let cutoff_24h = now - TimeDuration::hours(24);
    let cutoff_final_24h = DEADLINE - TimeDuration::hours(24);

    let mut authors: HashMap<String, AuthorAgg> = HashMap::new();
    let mut email_domains: HashMap<String, usize> = HashMap::new();
    let mut link_domains: HashMap<String, usize> = HashMap::new();
    let mut title_words: HashMap<String, usize> = HashMap::new();
    let mut description_words: HashMap<String, usize> = HashMap::new();
    let mut day_counts: HashMap<String, usize> = HashMap::new();
    let mut hour_counts: [usize; 24] = [0; 24];
    let mut desc_word_counts: Vec<usize> = Vec::with_capacity(rows.len());
    let mut title_word_total: usize = 0;
    let mut latest_iso: Option<&str> = None;

    for row in rows {
        let display = row.author.trim();
        let key = display.to_lowercase();
        if !key.is_empty() {
            let entry = authors.entry(key).or_insert_with(|| AuthorAgg {
                display: display.to_string(),
                emails: Vec::new(),
                count: 0,
            });
            entry.count += 1;
            let email = row.email.trim().to_lowercase();
            if !email.is_empty() && !entry.emails.iter().any(|e| e == &email) {
                entry.emails.push(email);
            }
        }

        if let Some(domain) = email_domain(&row.email) {
            *email_domains.entry(domain).or_insert(0) += 1;
        }
        if let Some(domain) = link_domain(&row.link) {
            *link_domains.entry(domain).or_insert(0) += 1;
        }

        for word in iter_words(&row.title) {
            *title_words.entry(word).or_insert(0) += 1;
            title_word_total += 1;
        }

        let mut desc_count = 0usize;
        for word in iter_words(&row.description) {
            *description_words.entry(word).or_insert(0) += 1;
            desc_count += 1;
        }
        desc_word_counts.push(desc_count);

        if let Ok(ts) = OffsetDateTime::parse(&row.created_at, &Iso8601::DEFAULT) {
            if ts >= cutoff_24h {
                stats.last_24h += 1;
            }
            if ts >= cutoff_final_24h {
                stats.final_24h_before_deadline += 1;
            }
            if let Some((day, _)) = row.created_at.split_once('T') {
                *day_counts.entry(day.to_string()).or_insert(0) += 1;
            }
            hour_counts[ts.hour() as usize] += 1;
        }

        match latest_iso {
            None => latest_iso = Some(&row.created_at),
            Some(prev) if row.created_at.as_str() > prev => latest_iso = Some(&row.created_at),
            _ => {}
        }
    }

    stats.distinct_authors = authors.len();
    let mut dups: Vec<DuplicateAuthor> = authors
        .into_values()
        .filter(|a| a.count > 1)
        .map(|a| DuplicateAuthor {
            name: a.display,
            count: a.count,
            emails: a.emails,
        })
        .collect();
    dups.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    stats.duplicate_authors_count = dups.len();
    stats.duplicate_authors = dups;
    stats.distinct_email_domains = email_domains.len();
    stats.distinct_link_domains = link_domains.len();
    stats.last_submission_at = latest_iso.map(str::to_string);

    desc_word_counts.sort_unstable();
    stats.desc_word_min = *desc_word_counts.first().unwrap_or(&0);
    stats.desc_word_max = *desc_word_counts.last().unwrap_or(&0);
    stats.desc_word_median = desc_word_counts
        .get(desc_word_counts.len() / 2)
        .copied()
        .unwrap_or(0);
    let desc_total: usize = desc_word_counts.iter().sum();
    stats.desc_word_avg = if rows.is_empty() {
        0
    } else {
        desc_total / rows.len()
    };
    stats.title_word_avg = if rows.is_empty() {
        0
    } else {
        title_word_total / rows.len()
    };

    stats.title_words = top_words(title_words, TOP_TITLE_WORDS);
    stats.description_words = top_words(description_words, TOP_DESC_WORDS);
    stats.email_domains = top_domains(email_domains, TOP_DOMAINS);
    stats.link_domains = top_domains(link_domains, TOP_DOMAINS);
    stats.timeline = build_timeline(day_counts);
    stats.hour_of_day = build_hours(hour_counts);

    stats
}

fn iter_words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .map(|w| w.to_lowercase())
        .filter(|w| {
            w.chars().count() >= MIN_WORD_LEN
                && !STOPWORDS.contains(&w.as_str())
                && w.chars().any(|c| c.is_alphabetic())
        })
}

fn top_words(map: HashMap<String, usize>, limit: usize) -> Vec<WordFreq> {
    let mut entries: Vec<(String, usize)> = map.into_iter().collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    entries.truncate(limit);
    let max = entries.first().map(|(_, c)| *c).unwrap_or(1).max(1);
    entries
        .into_iter()
        .map(|(word, count)| WordFreq {
            word,
            count,
            weight: count as f64 / max as f64,
        })
        .collect()
}

fn top_domains(map: HashMap<String, usize>, limit: usize) -> Vec<DomainCount> {
    let mut entries: Vec<(String, usize)> = map.into_iter().collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    entries.truncate(limit);
    let max = entries.first().map(|(_, c)| *c).unwrap_or(1).max(1);
    entries
        .into_iter()
        .map(|(domain, count)| DomainCount {
            domain,
            count,
            weight: count as f64 / max as f64,
        })
        .collect()
}

fn build_timeline(map: HashMap<String, usize>) -> Vec<DayCount> {
    let mut entries: Vec<(String, usize)> = map.into_iter().collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let max = entries.iter().map(|(_, c)| *c).max().unwrap_or(1).max(1);
    entries
        .into_iter()
        .map(|(day, count)| {
            let label = format_day(&day);
            DayCount {
                day,
                label,
                count,
                weight: count as f64 / max as f64,
            }
        })
        .collect()
}

fn build_hours(counts: [usize; 24]) -> Vec<HourCount> {
    let max = *counts.iter().max().unwrap_or(&1).max(&1);
    counts
        .iter()
        .enumerate()
        .map(|(hour, &count)| HourCount {
            hour: hour as u8,
            label: format!("{hour:02}"),
            count,
            weight: count as f64 / max as f64,
        })
        .collect()
}

fn format_day(iso_day: &str) -> String {
    let Some((year, rest)) = iso_day.split_once('-') else {
        return iso_day.to_string();
    };
    let Some((month, day)) = rest.split_once('-') else {
        return iso_day.to_string();
    };
    let month_name = match month {
        "01" => "Jan",
        "02" => "Feb",
        "03" => "Mar",
        "04" => "Apr",
        "05" => "May",
        "06" => "Jun",
        "07" => "Jul",
        "08" => "Aug",
        "09" => "Sep",
        "10" => "Oct",
        "11" => "Nov",
        "12" => "Dec",
        _ => return iso_day.to_string(),
    };
    let day_num = day.trim_start_matches('0');
    let _ = year;
    format!("{month_name} {day_num}")
}

fn email_domain(email: &str) -> Option<String> {
    let (_, domain) = email.trim().split_once('@')?;
    let domain = domain.trim().trim_end_matches('.').to_lowercase();
    if domain.is_empty() || !domain.contains('.') {
        return None;
    }
    Some(domain)
}

fn link_domain(link: &str) -> Option<String> {
    let url = Url::parse(link.trim()).ok()?;
    let host = url.host_str()?.to_lowercase();
    Some(host.trim_start_matches("www.").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_words_skipping_stopwords_and_short() {
        let words: Vec<String> =
            iter_words("The quick, brown fox jumps over a lazy dog.").collect();
        // "the", "a", "over" are stopwords; commas/periods become delimiters.
        assert_eq!(words, vec!["quick", "brown", "fox", "jumps", "lazy", "dog"]);
    }

    #[test]
    fn extracts_unicode_words_lowercased() {
        let words: Vec<String> = iter_words("Café résumé naïve").collect();
        assert_eq!(words, vec!["café", "résumé", "naïve"]);
    }

    #[test]
    fn contractions_split_on_apostrophe() {
        let words: Vec<String> = iter_words("don't won't they're Anna's project").collect();
        // contraction roots ("don", "won", "they", "re", "s") are stopwords or too short;
        // possessive base "anna" survives, plus "project".
        assert_eq!(words, vec!["anna", "project"]);
    }

    #[test]
    fn email_domain_parsing() {
        assert_eq!(email_domain("a@b.co"), Some("b.co".into()));
        assert_eq!(email_domain("  Foo@Gmail.COM  "), Some("gmail.com".into()));
        assert_eq!(email_domain("no-at"), None);
        assert_eq!(email_domain("a@nodomain"), None);
    }

    #[test]
    fn link_domain_parsing() {
        assert_eq!(
            link_domain("https://www.example.com/path"),
            Some("example.com".into())
        );
        assert_eq!(
            link_domain("http://Substack.COM/foo"),
            Some("substack.com".into())
        );
        assert_eq!(link_domain("not a url"), None);
    }

    #[test]
    fn top_words_orders_by_count_then_alpha() {
        let mut m = HashMap::new();
        m.insert("apple".to_string(), 3);
        m.insert("banana".to_string(), 3);
        m.insert("cherry".to_string(), 1);
        let top = top_words(m, 10);
        assert_eq!(top[0].word, "apple");
        assert_eq!(top[1].word, "banana");
        assert_eq!(top[2].word, "cherry");
        assert!((top[0].weight - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn format_day_examples() {
        assert_eq!(format_day("2026-04-26"), "Apr 26");
        assert_eq!(format_day("2026-01-05"), "Jan 5");
    }
}
