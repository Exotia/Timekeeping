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
    // A spreadsheet on Windows writes a byte-order mark before the header;
    // left in place it glues itself to the first column name.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
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
    // A blank line before the header is a stray newline, not an empty file:
    // skipping to the first record with content keeps the rows behind it.
    let mut records = split_records(text)
        .into_iter()
        .filter(|(_, f)| !is_blank(f));
    let Some((_, header)) = records.next() else {
        return Ok(Vec::new());
    };
    let layout = layout_of(&header).ok_or(ImportError::Header)?;
    let mut rows = Vec::new();
    for (line, f) in records {
        let get = |idx: usize| f.get(idx).map(String::as_str).unwrap_or_default();
        rows.push(row_from(
            line,
            get(layout.date),
            get(layout.start),
            get(layout.end),
            get(layout.project),
            get(layout.comment),
        )?);
    }
    Ok(rows)
}

fn is_blank(fields: &[String]) -> bool {
    fields.iter().all(|f| f.trim().is_empty())
}

/// The text as CSV records of fields, each with the 1-based line it starts on.
///
/// A record is usually a line, but `csv_quote` wraps a comment containing a
/// newline in quotes and writes the newline literally, so a record ends at a
/// newline only outside quotes. `""` is an escaped quote, and a `\r` before a
/// newline is a line ending rather than data.
fn split_records(text: &str) -> Vec<(usize, Vec<String>)> {
    let mut out = Vec::new();
    let mut fields: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut line = 1usize;
    let mut start = 1usize;
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '"' if in_quotes && it.peek() == Some(&'"') => {
                cur.push('"');
                it.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => fields.push(std::mem::take(&mut cur)),
            '\r' if !in_quotes && it.peek() == Some(&'\n') => {}
            '\n' if !in_quotes => {
                fields.push(std::mem::take(&mut cur));
                out.push((start, std::mem::take(&mut fields)));
                line += 1;
                start = line;
            }
            _ => {
                if c == '\n' {
                    line += 1;
                }
                cur.push(c);
            }
        }
    }
    if !cur.is_empty() || !fields.is_empty() {
        fields.push(cur);
        out.push((start, fields));
    }
    out
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

/// The flat array `tk export --format json` writes. Hand-rolled to match the
/// hand-rolled writer; the shape is fixed and shallow.
fn parse_json(text: &str) -> Result<Vec<ImportRow>, ImportError> {
    let mut rows = Vec::new();
    for (i, obj) in json_objects(text).iter().enumerate() {
        rows.push(row_from(
            i + 1,
            &json_field(obj, "date"),
            &json_field(obj, "start"),
            &json_field(obj, "end"),
            &json_field(obj, "project"),
            &json_field(obj, "comment"),
        )?);
    }
    Ok(rows)
}

/// The array's top-level objects, found by brace depth with string literals
/// skipped — a `{`, `}` or `,` inside a comment must not split one.
fn json_objects(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    let mut in_str = false;
    let mut esc = false;
    for c in text.chars() {
        if in_str {
            if depth > 0 {
                cur.push(c);
            }
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                if depth > 0 {
                    cur.push(c);
                }
            }
            '{' => {
                depth += 1;
                if depth == 1 {
                    cur.clear();
                } else {
                    cur.push(c);
                }
            }
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    out.push(std::mem::take(&mut cur));
                } else {
                    cur.push(c);
                }
            }
            _ => {
                if depth > 0 {
                    cur.push(c);
                }
            }
        }
    }
    out
}

/// One string-valued field of an object, unescaping what `json_quote` wrote.
fn json_field(obj: &str, name: &str) -> String {
    let needle = format!("\"{name}\":");
    let Some(p) = obj.find(&needle) else {
        return String::new();
    };
    let rest = obj[p + needle.len()..].trim_start();
    let mut it = rest.chars();
    if it.next() != Some('"') {
        return String::new();
    }
    let mut out = String::new();
    let mut esc = false;
    for c in it {
        if esc {
            out.push(match c {
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                other => other,
            });
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == '"' {
            break;
        } else {
            out.push(c);
        }
    }
    out
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

    // Finding 1 (critical): the writer escapes a quote as \", and the reader
    // used to stop at that escaped quote and truncate the comment.
    #[test]
    fn a_json_comment_keeps_its_quotes() {
        let text = r#"[{"date":"2026-09-14","start":"09:00","end":"15:30","project":"Alpha","comment":"say \"hi\" now","gross_minutes":390}]"#;
        let rows = parse_import(text).unwrap();
        assert_eq!(rows[0].comment, "say \"hi\" now");
    }

    // Finding 2 (critical): one stray newline used to discard every row and
    // still report success.
    #[test]
    fn a_blank_first_line_does_not_discard_the_file() {
        let text = "\n\
                    date,start,end,project,comment,gross,net,break\n\
                    2026-09-14,09:00,15:30,Alpha,note,06:30,05:42,00:48\n";
        let rows = parse_import(text).unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].project, "Alpha");
    }

    // Finding 3: csv_quote puts a real newline inside a quoted field, so a
    // record is not always a line.
    #[test]
    fn a_quoted_comment_may_span_lines() {
        let text = "date,start,end,project,comment,gross,net,break\n\
                    2026-09-14,09:00,15:30,Alpha,\"line one\nline two\",06:30,05:42,00:48\n";
        let rows = parse_import(text).unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].comment, "line one\nline two");
    }

    // Finding 4: Excel on Windows writes a BOM as well as CRLF.
    #[test]
    fn a_utf8_bom_does_not_hide_the_header() {
        let text = "\u{feff}date,start,end,project,comment,gross,net,break\r\n\
                    2026-09-14,09:00,15:30,Alpha,note,06:30,05:42,00:48\r\n";
        let rows = parse_import(text).unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].project, "Alpha");
    }

    // Finding 5: splitting objects on the literal "}," broke on a comment
    // that contained one.
    #[test]
    fn a_json_comment_containing_a_brace_comma_parses() {
        let text = r#"[{"date":"2026-09-14","start":"09:00","end":"15:30","project":"Alpha","comment":"fix }, then go","gross_minutes":390}]"#;
        let rows = parse_import(text).unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].comment, "fix }, then go");
        assert_eq!(rows[0].project, "Alpha");
    }

    #[test]
    fn two_json_objects_still_parse() {
        let text = r#"[{"date":"2026-09-14","start":"09:00","end":"15:30","project":"Alpha","comment":"a"},{"date":"2026-09-15","start":"09:00","end":"17:00","project":"Beta","comment":"b"}]"#;
        let rows = parse_import(text).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].project, "Beta");
    }
}
