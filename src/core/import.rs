//! Reading entries back in: `tk export`'s own CSV and JSON, and the CSV the
//! Python tool this project replaced used to write.
//!
//! The `gross`/`net`/`break` columns (`brutto`/`netto` in the old tool) are
//! parsed past and thrown away. They are derived from the break tiers, so
//! trusting a file's copy of them would let an import contradict the config.

use chrono::{NaiveDate, NaiveTime};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRow {
    pub date: NaiveDate,
    pub start: NaiveTime,
    pub end: NaiveTime,
    pub project: String,
    pub comment: String,
    /// 1-based line in the source file, so a skip can say which row it was.
    pub line: usize,
}

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("line {line}: {what}")]
    Row { line: usize, what: String },
    #[error("unrecognised header; expected a tk export or the old tool's CSV")]
    Header,
}

/// Detect the format from the text itself and parse every row.
pub fn parse_import(text: &str) -> Result<Vec<ImportRow>, ImportError> {
    if text.trim_start().starts_with('[') {
        return parse_json(text);
    }
    parse_csv(text)
}

/// Column indices for the fields we keep, resolved from the header.
struct Layout {
    date: usize,
    start: usize,
    end: usize,
    project: usize,
    comment: usize,
}

fn layout_of(header: &[String]) -> Option<Layout> {
    let at = |name: &str| header.iter().position(|h| h == name);
    // tk's own export, and the old Python tool's, differ in order and in the
    // names of the two time columns; everything else we want is common.
    let start = at("start").or_else(|| at("start_time"))?;
    let end = at("end").or_else(|| at("end_time"))?;
    Some(Layout {
        date: at("date")?,
        start,
        end,
        project: at("project")?,
        comment: at("comment")?,
    })
}

fn parse_csv(text: &str) -> Result<Vec<ImportRow>, ImportError> {
    let mut lines = text.lines().enumerate();
    let Some((_, header_line)) = lines.next() else {
        return Ok(Vec::new());
    };
    if header_line.trim().is_empty() {
        return Ok(Vec::new());
    }
    let layout = layout_of(&split_csv(header_line)).ok_or(ImportError::Header)?;
    let mut rows = Vec::new();
    for (i, line) in lines {
        let line_no = i + 1;
        if line.trim().is_empty() {
            continue;
        }
        let f = split_csv(line);
        let get = |idx: usize| f.get(idx).map(String::as_str).unwrap_or_default();
        rows.push(row_from(
            line_no,
            get(layout.date),
            get(layout.start),
            get(layout.end),
            get(layout.project),
            get(layout.comment),
        )?);
    }
    Ok(rows)
}

fn row_from(
    line: usize,
    date: &str,
    start: &str,
    end: &str,
    project: &str,
    comment: &str,
) -> Result<ImportRow, ImportError> {
    let bad = |what: String| ImportError::Row { line, what };
    let time = |s: &str| {
        NaiveTime::parse_from_str(s, "%H:%M").map_err(|_| bad(format!("'{s}' is not a HH:MM time")))
    };
    if project.trim().is_empty() {
        return Err(bad("no project".into()));
    }
    Ok(ImportRow {
        date: NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map_err(|_| bad(format!("'{date}' is not a YYYY-MM-DD date")))?,
        start: time(start)?,
        end: time(end)?,
        project: project.trim().to_string(),
        comment: comment.trim().to_string(),
        line,
    })
}

/// One CSV line into fields, honouring the `""`-escaped quoting `csv_quote`
/// writes. A trailing `\r` is a line ending, not data.
fn split_csv(line: &str) -> Vec<String> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// The flat array `tk export --format json` writes. Hand-rolled to match the
/// hand-rolled writer; the shape is fixed and shallow.
fn parse_json(text: &str) -> Result<Vec<ImportRow>, ImportError> {
    let mut rows = Vec::new();
    for (i, obj) in text
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split("},")
        .enumerate()
    {
        let obj = obj.trim().trim_start_matches('{').trim_end_matches('}');
        if obj.trim().is_empty() {
            continue;
        }
        let line = i + 1;
        let field = |name: &str| -> String {
            let needle = format!("\"{name}\":\"");
            match obj.find(&needle) {
                Some(p) => {
                    let rest = &obj[p + needle.len()..];
                    let end = rest.find('"').unwrap_or(rest.len());
                    rest[..end].replace("\\\"", "\"").replace("\\\\", "\\")
                }
                None => String::new(),
            }
        };
        rows.push(row_from(
            line,
            &field("date"),
            &field("start"),
            &field("end"),
            &field("project"),
            &field("comment"),
        )?);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn reads_the_tk_export_header() {
        let text = "date,start,end,project,comment,gross,net,break\n\
                    2026-09-14,09:00,15:30,Alpha,note,06:30,05:42,00:48\n";
        let rows = parse_import(text).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].date, d(2026, 9, 14));
        assert_eq!(rows[0].project, "Alpha");
        assert_eq!(rows[0].comment, "note");
        assert_eq!(rows[0].line, 2);
    }

    #[test]
    fn reads_the_legacy_python_header() {
        // Different column order, and the derived columns are named in German.
        let text = "date,project,start_time,end_time,brutto,netto,comment\n\
                    2026-09-14,Alpha,09:00,15:30,6.5,5.7,note\n";
        let rows = parse_import(text).unwrap();
        assert_eq!(rows[0].project, "Alpha");
        assert_eq!(rows[0].start, NaiveTime::from_hms_opt(9, 0, 0).unwrap());
        assert_eq!(rows[0].comment, "note");
    }

    #[test]
    fn reads_json() {
        let text = r#"[{"date":"2026-09-14","start":"09:00","end":"15:30","project":"Alpha","comment":"note","gross_minutes":390,"net_minutes":342,"break_minutes":48}]"#;
        let rows = parse_import(text).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].project, "Alpha");
    }

    // Review Focus 1: a spreadsheet on Windows writes CRLF.
    #[test]
    fn crlf_does_not_stick_to_the_last_field() {
        let text = "date,start,end,project,comment,gross,net,break\r\n\
                    2026-09-14,09:00,15:30,Alpha,note,06:30,05:42,00:48\r\n";
        let rows = parse_import(text).unwrap();
        assert_eq!(rows[0].comment, "note");
    }

    // Review Focus 2: csv_quote emits these on export, so they must come back.
    #[test]
    fn quoted_fields_keep_their_commas_and_quotes() {
        let text = "date,start,end,project,comment,gross,net,break\n\
                    2026-09-14,09:00,15:30,Alpha,\"lunch, then \"\"review\"\"\",06:30,05:42,00:48\n";
        let rows = parse_import(text).unwrap();
        assert_eq!(rows[0].comment, "lunch, then \"review\"");
    }

    #[test]
    fn a_bad_row_names_its_line() {
        let text = "date,start,end,project,comment,gross,net,break\n\
                    2026-09-14,not-a-time,15:30,Alpha,,,,\n";
        let err = parse_import(text).unwrap_err();
        assert!(err.to_string().contains("line 2"), "{err}");
    }

    #[test]
    fn an_unknown_header_is_refused() {
        let err = parse_import("alpha,beta\n1,2\n").unwrap_err();
        assert!(err.to_string().contains("unrecognised"), "{err}");
    }

    // Review Focus 4, parser half: no rows is not an error.
    #[test]
    fn a_header_with_no_rows_is_empty_not_an_error() {
        let text = "date,start,end,project,comment,gross,net,break\n";
        assert!(parse_import(text).unwrap().is_empty());
        assert!(parse_import("").unwrap().is_empty());
    }
}
