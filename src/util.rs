pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn new_id(prefix: &str) -> String {
    let u = uuid::Uuid::new_v4().simple().to_string();
    format!("{prefix}_{}", &u[..16])
}

/// Truncate to at most `max` bytes on a char boundary, appending a marker when cut.
pub fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n…[truncated {} bytes]", &s[..end], s.len() - end)
}

/// Shorten a human-facing label to `max` characters with an ellipsis (no byte counts).
pub fn ellipsis(s: &str, max: usize) -> String {
    let s = s.trim().replace('\n', " ");
    if s.chars().count() <= max {
        return s;
    }
    format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>().trim_end())
}

/// Keep the last `max_lines` lines of `s`.
pub fn tail_lines(s: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    if lines.len() <= max_lines {
        return s.to_string();
    }
    format!("…[{} earlier lines omitted]\n{}", lines.len() - max_lines, lines[lines.len() - max_lines..].join("\n"))
}

/// Quote a string for POSIX `sh` (used only for commands executed inside the sandbox).
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn years_since(rfc3339: &str) -> Option<f64> {
    let t = chrono::DateTime::parse_from_rfc3339(rfc3339).ok()?;
    let d = chrono::Utc::now().signed_duration_since(t.with_timezone(&chrono::Utc));
    Some((d.num_days() as f64 / 365.25 * 10.0).round() / 10.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_respects_char_boundaries() {
        let s = "ééééé";
        let t = truncate(s, 3);
        assert!(t.starts_with('é'));
        assert!(t.contains("truncated"));
    }

    #[test]
    fn sh_quote_escapes_single_quotes() {
        assert_eq!(sh_quote("a'b"), "'a'\\''b'");
    }
}
