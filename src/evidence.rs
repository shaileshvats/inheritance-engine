//! Evidence handling: presenting files to agents with line numbers, and independently checking
//! every citation an agent returns against the real file on disk.

use std::path::Path;

use crate::contracts::Evidence;
use crate::security::{redact, safe_join};

/// Render `text` with `N| ` line prefixes for the inclusive 1-based range.
pub fn number_lines(text: &str, start: usize, end: usize) -> String {
    let mut out = String::new();
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        if n < start {
            continue;
        }
        if n > end {
            break;
        }
        out.push_str(&format!("{n}| {line}\n"));
    }
    out
}

/// A file excerpt prepared for a prompt. When the file is long, windows around `focus` lines are used.
pub struct Excerpt {
    pub path: String,
    pub body: String,
    pub total_lines: usize,
    pub shown: String,
}

pub fn excerpt(root: &Path, path: &str, focus: &[usize], max_chars: usize) -> Option<Excerpt> {
    let full = safe_join(root, path).ok()?;
    let text = std::fs::read_to_string(full).ok()?;
    let text = redact(&text);
    let total = text.lines().count();
    let whole = number_lines(&text, 1, total);
    if whole.len() <= max_chars {
        return Some(Excerpt { path: path.into(), body: whole, total_lines: total, shown: format!("1-{total}") });
    }
    // Windows of +/- 30 lines around each focus line (or the head of the file), merged.
    let mut ranges: Vec<(usize, usize)> = if focus.is_empty() { vec![(1, 120)] } else { focus.iter().map(|&l| (l.saturating_sub(30).max(1), (l + 30).min(total))).collect() };
    ranges.sort();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for r in ranges {
        match merged.last_mut() {
            Some(last) if r.0 <= last.1 + 1 => last.1 = last.1.max(r.1),
            _ => merged.push(r),
        }
    }
    let mut body = String::new();
    let mut shown = Vec::new();
    for (a, b) in merged {
        let chunk = number_lines(&text, a, b);
        if body.len() + chunk.len() > max_chars {
            break;
        }
        if !body.is_empty() {
            body.push_str("…\n");
        }
        body.push_str(&chunk);
        shown.push(format!("{a}-{b}"));
    }
    if body.is_empty() {
        body = crate::util::truncate(&number_lines(&text, 1, 120), max_chars);
        shown.push("1-120".into());
    }
    Some(Excerpt { path: path.into(), body, total_lines: total, shown: shown.join(",") })
}

fn normalize(s: &str) -> String {
    // Drop any "N| " prefixes the model may have copied, then collapse whitespace.
    let mut out = String::new();
    for line in s.lines() {
        let l = line.trim_start();
        let l = match l.find("| ") {
            Some(i) if l[..i].chars().all(|c| c.is_ascii_digit()) && i > 0 => &l[i + 2..],
            _ => l,
        };
        out.push_str(l);
        out.push(' ');
    }
    let collapsed: String = out.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.trim_end_matches("...").trim_end_matches('…').trim().to_string()
}

/// Check one citation in place. Line numbers are corrected when the exact quote exists elsewhere in the file.
pub fn check(root: &Path, ev: &mut Evidence) {
    let Ok(full) = safe_join(root, &ev.path) else {
        ev.check = "missing_file".into();
        return;
    };
    let Ok(text) = std::fs::read_to_string(full) else {
        ev.check = "missing_file".into();
        return;
    };
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    if ev.start_line == 0 {
        ev.start_line = 1;
    }
    if ev.end_line < ev.start_line {
        ev.end_line = ev.start_line;
    }
    let quote = normalize(&ev.quote);
    if quote.chars().count() < 4 {
        ev.check = if ev.start_line <= total { "unchecked".into() } else { "out_of_range".into() };
        if ev.start_line <= total {
            ev.quote = lines[ev.start_line - 1..ev.end_line.min(total)].join("\n");
        }
        return;
    }
    if ev.start_line <= total {
        let lo = ev.start_line.saturating_sub(2).max(1);
        let hi = (ev.end_line + 2).min(total);
        let window = normalize(&lines[lo - 1..hi].join("\n"));
        if window.contains(&quote) {
            ev.check = "verified".into();
            return;
        }
    }
    // Search the whole file for the quote and relocate the citation if found.
    // Smallest window that contains the quote (earliest among equals).
    let qlines = ev.quote.lines().count().max(1);
    for len in 1..=(qlines + 2) {
        for start in 0..total {
            let end = start + len;
            if end > total {
                break;
            }
            if normalize(&lines[start..end].join("\n")).contains(&quote) {
                ev.start_line = start + 1;
                ev.end_line = end;
                ev.check = "verified".into();
                return;
            }
        }
    }
    ev.check = if ev.start_line > total { "out_of_range".into() } else { "mismatch".into() };
}

pub fn check_all(root: &Path, evs: &mut [Evidence]) {
    for e in evs.iter_mut() {
        check(root, e);
    }
}

/// The engine, not the model, decides whether a claim may be called "verified".
pub fn enforce_status(model_status: &str, evs: &[Evidence]) -> String {
    let any_verified = evs.iter().any(|e| e.check == "verified");
    match model_status {
        "verified" if any_verified => "verified".into(),
        "verified" | "inferred" => "inferred".into(),
        _ => "unknown".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("m.py"), "import os\n\ndef merge(a, b):\n    return dict(a, **b)\n").unwrap();
        d
    }

    fn ev(path: &str, s: usize, e: usize, q: &str) -> Evidence {
        Evidence { path: path.into(), start_line: s, end_line: e, quote: q.into(), check: "unchecked".into() }
    }

    #[test]
    fn verifies_exact_and_relocates() {
        let d = setup();
        let mut a = ev("m.py", 3, 4, "def merge(a, b):");
        check(d.path(), &mut a);
        assert_eq!(a.check, "verified");
        let mut b = ev("m.py", 1, 1, "return dict(a, **b)");
        check(d.path(), &mut b);
        assert_eq!(b.check, "verified");
        assert_eq!(b.start_line, 4);
        let mut c = ev("m.py", 3, 3, "4|     return dict(a, **b)");
        check(d.path(), &mut c);
        assert_eq!(c.check, "verified");
    }

    #[test]
    fn flags_fabrications() {
        let d = setup();
        let mut a = ev("m.py", 3, 3, "def fabricate_everything():");
        check(d.path(), &mut a);
        assert_eq!(a.check, "mismatch");
        let mut b = ev("nope.py", 1, 1, "x");
        check(d.path(), &mut b);
        assert_eq!(b.check, "missing_file");
        let mut c = ev("../etc/passwd", 1, 1, "root");
        check(d.path(), &mut c);
        assert_eq!(c.check, "missing_file");
    }

    #[test]
    fn status_is_downgraded_without_verified_evidence() {
        let bad = vec![Evidence { path: "a".into(), start_line: 1, end_line: 1, quote: "x".into(), check: "mismatch".into() }];
        assert_eq!(enforce_status("verified", &bad), "inferred");
        let good = vec![Evidence { path: "a".into(), start_line: 1, end_line: 1, quote: "x".into(), check: "verified".into() }];
        assert_eq!(enforce_status("verified", &good), "verified");
        assert_eq!(enforce_status("unknown", &good), "unknown");
    }

    #[test]
    fn excerpt_windows_long_files() {
        let d = tempfile::tempdir().unwrap();
        let body: String = (1..=1000).map(|i| format!("line {i}\n")).collect();
        std::fs::write(d.path().join("big.py"), body).unwrap();
        let e = excerpt(d.path(), "big.py", &[500], 4000).unwrap();
        assert!(e.body.contains("500| line 500"));
        assert!(!e.body.contains("1| line 1\n"));
        assert_eq!(e.total_lines, 1000);
    }
}
