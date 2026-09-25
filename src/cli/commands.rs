use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, anyhow, bail};
use chrono::{Days, Local, NaiveDate, NaiveDateTime, Timelike};

use super::{Command, Ctx, ProjectAction};
use crate::config::{Config, ConfigPatch};
use crate::core::import::parse_import;
use crate::core::{
    BreakTier, DayKind, Entry, HoursFormat, Minutes, TodayCtx, day_stats, entry_nets, parse_date,
    parse_time_range, provisional_net_for, running_balance, running_minutes,
};
use crate::store::ImportReport;

pub fn now_local() -> NaiveDateTime {
    Local::now().naive_local()
}

/// Copy the database to `<home>/backups/tk-YYYYmmdd-HHMMSS.db` and return the path.
pub fn write_backup(ctx: &Ctx, now: NaiveDateTime) -> anyhow::Result<PathBuf> {
    let dir = ctx.home.join("backups");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("tk-{}.db", now.format("%Y%m%d-%H%M%S")));
    // `VACUUM INTO` refuses an existing target, so a second backup within the
    // same second (two presses of `B`, say) would otherwise surface a raw
    // SQLite error instead of just refreshing the file.
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    ctx.store.backup_to(&path)?;
    Ok(path)
}

/// The net of one entry of a day: its gross minus its share of its session's
/// break deduction.
///
/// `entries` is the whole day, because an entry's share depends on the session
/// it sits in and on the other entries of that session. An entry that is not in
/// the day at all keeps its gross, which cannot happen for one just written.
fn net_of(entries: &[Entry], entry: &Entry, tiers: &[BreakTier]) -> Minutes {
    entries
        .iter()
        .position(|e| e.id == entry.id)
        .and_then(|i| entry_nets(entries, tiers).get(i).copied())
        .unwrap_or_else(|| entry.duration())
}

/// Net per entry for a run of entries spanning several days, aligned with
/// `entries`: sessions live inside one day, so the entries are grouped by date
/// before their shares are worked out.
fn nets_by_day(entries: &[Entry], tiers: &[BreakTier]) -> Vec<Minutes> {
    let mut by_date: std::collections::BTreeMap<NaiveDate, Vec<usize>> = Default::default();
    for (i, e) in entries.iter().enumerate() {
        by_date.entry(e.date).or_default().push(i);
    }
    let mut out = vec![Minutes::ZERO; entries.len()];
    for idx in by_date.into_values() {
        let day: Vec<Entry> = idx.iter().map(|&i| entries[i].clone()).collect();
        for (k, net) in entry_nets(&day, tiers).into_iter().enumerate() {
            out[idx[k]] = net;
        }
    }
    out
}

pub fn balance_as_of(ctx: &Ctx, today: NaiveDate, clocked_in: bool) -> anyhow::Result<Minutes> {
    let rules = ctx.config.rules();
    let cal = ctx.config.calendar();
    let days = ctx.store.days_in(rules.start_date, today, &cal)?;
    let tc = TodayCtx { today, clocked_in };
    let stats: Vec<_> = days
        .iter()
        .map(|d| day_stats(d, &rules, &cal, &tc))
        .collect();
    Ok(running_balance(&stats, &rules))
}

pub fn status_line(ctx: &Ctx, now: NaiveDateTime) -> anyhow::Result<String> {
    let today = now.date();
    let f = ctx.config.hours_format();
    let rules = ctx.config.rules();
    let session = ctx.store.session()?;
    let balance = balance_as_of(ctx, today, session.is_some())?;
    let entries = ctx.store.entries_on(today)?;
    Ok(match session {
        // On a break the work so far is already booked, so today's net is the
        // day's own net; what is running is the break itself.
        Some(s) if s.state.is_break() => {
            let paused = running_minutes(NaiveDateTime::new(s.date, s.start), now);
            let net = day_net(ctx, today, true)?;
            format!(
                "☕ on break {} (since {}){} · today {} · balance {}",
                paused.fmt_unsigned(f),
                s.start.format("%H:%M"),
                s.project
                    .as_deref()
                    .map(|p| format!(" · {p}"))
                    .unwrap_or_default(),
                net.fmt_signed(f),
                balance.fmt_signed(f)
            )
        }
        Some(s) => {
            // The session carries its own date, so a clock-in from yesterday keeps
            // counting instead of wrapping back to 00:00 at midnight.
            let running = running_minutes(NaiveDateTime::new(s.date, s.start), now);
            let net = provisional_net_for(&entries, s.start, running, &rules);
            // A session recorded before `tk` stored the project simply has none to name.
            let project = s
                .project
                .as_deref()
                .map(|p| format!("{p} "))
                .unwrap_or_default();
            format!(
                "⏱ {}{} (in {}) · today {} · balance {}",
                project,
                running.fmt_unsigned(f),
                s.start.format("%H:%M"),
                net.fmt_signed(f),
                balance.fmt_signed(f)
            )
        }
        None => {
            let net = day_net(ctx, today, false)?;
            format!(
                "not clocked in · today {} · balance {}",
                net.fmt_signed(f),
                balance.fmt_signed(f)
            )
        }
    })
}

/// The net of one day as the books stand, without a running clock in it.
fn day_net(ctx: &Ctx, date: NaiveDate, clocked_in: bool) -> anyhow::Result<Minutes> {
    let cal = ctx.config.calendar();
    let d = ctx.store.days_in(date, date, &cal)?.remove(0);
    Ok(day_stats(
        &d,
        &ctx.config.rules(),
        &cal,
        &TodayCtx {
            today: date,
            clocked_in,
        },
    )
    .net)
}

pub fn run(cmd: Command, ctx: &Ctx, out: &mut dyn Write) -> anyhow::Result<()> {
    let now = now_local();
    let today = now.date();
    // How this run spells durations. Export is deliberately not part of it.
    let f: HoursFormat = ctx.config.hours_format();
    match cmd {
        Command::In { force, project } => {
            // Clocking in on a break is how the user comes back to work, so it
            // resumes the session rather than complaining that one is open.
            if !force
                && let Some(s) = ctx.store.session()?
                && s.state.is_break()
            {
                let s = ctx.store.resume(now, project.as_deref())?;
                writeln!(
                    out,
                    "Resumed {} at {}",
                    s.project.as_deref().unwrap_or("the last project"),
                    s.start.format("%H:%M")
                )?;
                return Ok(());
            }
            // An open session is a more useful complaint than a missing project, so it
            // comes first — unless `--force` is about to replace it anyway. The store
            // checks it again under its own lock, so a concurrent `tk in` is refused.
            if !force && let Some(s) = ctx.store.session()? {
                bail!(
                    "already clocked in since {} {}",
                    s.date,
                    s.start.format("%H:%M")
                );
            }
            // Resolved before `--force` discards anything: a clock-in that cannot name
            // its project must not have thrown away the session that was running.
            let project = match project.or(ctx.store.last_used_project()?) {
                Some(p) => p,
                None => bail!("no project yet; pass --project NAME"),
            };
            if force {
                ctx.store.clear_session()?;
            }
            let t = now.time().with_second(0).unwrap_or(now.time());
            ctx.store.clock_in(today, t, &project)?;
            writeln!(out, "Clocked in on {project} at {}", t.format("%H:%M"))?;
        }
        Command::Break { comment } => {
            let e = ctx
                .store
                .take_break(now, comment.as_deref().unwrap_or(""))?;
            let since = ctx
                .store
                .session()?
                .map(|s| s.start)
                .unwrap_or(e.end)
                .format("%H:%M")
                .to_string();
            writeln!(
                out,
                "Booked {}–{} {} ({}) · on break since {since}",
                e.start.format("%H:%M"),
                e.end.format("%H:%M"),
                e.project,
                e.duration().fmt_signed(f),
            )?;
        }
        Command::Switch { project, comment } => {
            let (e, s) =
                ctx.store
                    .switch_project(now, &project, comment.as_deref().unwrap_or(""))?;
            let net = net_of(
                &ctx.store.entries_on(e.date)?,
                &e,
                &ctx.config.rules().tiers,
            );
            writeln!(
                out,
                "Booked {}–{} {} ({}) · net {} · now on {} since {}",
                e.start.format("%H:%M"),
                e.end.format("%H:%M"),
                e.project,
                e.duration().fmt_signed(f),
                net.fmt_signed(f),
                s.project.as_deref().unwrap_or(&project),
                s.start.format("%H:%M")
            )?;
        }
        Command::Out { project, comment } => {
            // Read before the write: only the session that is about to be closed
            // knows whether it was running or paused, and since when.
            let before = ctx.store.session()?;
            // One transaction in the store: the entry and the cleared session, or neither.
            let booked = ctx.store.clock_out_with(
                now,
                project.as_deref(),
                comment.as_deref().unwrap_or(""),
            )?;
            let Some(e) = booked else {
                // A break was ended: the work before it was booked when it began.
                let since = before.expect("the store had a session to close");
                let paused = running_minutes(NaiveDateTime::new(since.date, since.start), now);
                writeln!(
                    out,
                    "Break ended after {} · nothing to book",
                    paused.fmt_unsigned(f)
                )?;
                return Ok(());
            };
            let rules = ctx.config.rules();
            let day = ctx
                .store
                .days_in(e.date, e.date, &ctx.config.calendar())?
                .remove(0);
            let net = net_of(&day.entries, &e, &rules.tiers);
            let stats = day_stats(
                &day,
                &rules,
                &ctx.config.calendar(),
                &TodayCtx {
                    today,
                    clocked_in: false,
                },
            );
            writeln!(
                out,
                "Recorded {}–{} {} ({}) · net {} · day net {} · balance {}",
                e.start.format("%H:%M"),
                e.end.format("%H:%M"),
                e.project,
                e.duration().fmt_signed(f),
                net.fmt_signed(f),
                stats.net.fmt_signed(f),
                balance_as_of(ctx, today, false)?.fmt_signed(f)
            )?;
        }
        Command::Status => writeln!(out, "{}", status_line(ctx, now)?)?,
        Command::Add {
            date,
            range,
            project,
            comment,
        } => {
            let date = parse_date(&date, today)?;
            let (s, e) = parse_time_range(&range)?;
            let entry = ctx.store.add_entry(date, s, e, &project, &comment)?;
            let day = ctx
                .store
                .days_in(date, date, &ctx.config.calendar())?
                .remove(0);
            let rules = ctx.config.rules();
            let net = net_of(&day.entries, &entry, &rules.tiers);
            let st = day_stats(
                &day,
                &rules,
                &ctx.config.calendar(),
                &TodayCtx {
                    today,
                    clocked_in: false,
                },
            );
            writeln!(
                out,
                "Added {} {}–{} {} · gross {} · net {} · day net {}",
                date,
                entry.start.format("%H:%M"),
                entry.end.format("%H:%M"),
                entry.project,
                entry.duration().fmt_signed(f),
                net.fmt_signed(f),
                st.net.fmt_signed(f)
            )?;
        }
        Command::Day {
            date,
            kind,
            to,
            label,
        } => {
            let from = parse_date(&date, today)?;
            let to = match to {
                Some(t) => parse_date(&t, today)?,
                None => from,
            };
            if to < from {
                bail!("--to must not be before DATE");
            }
            let kind = DayKind::parse(&kind, label.as_deref()).ok_or_else(|| {
                anyhow!("unknown kind '{kind}'; use work|vacation|flex|holiday|sick|absence")
            })?;
            let mut d = from;
            let mut n = 0;
            while d <= to {
                ctx.store
                    .set_day_kind(d, &kind)
                    .with_context(|| format!("{d}"))?;
                n += 1;
                d = d.checked_add_days(Days::new(1)).unwrap();
            }
            writeln!(
                out,
                "Set {} day{} to {}",
                n,
                if n == 1 { "" } else { "s" },
                kind.display_name().to_lowercase()
            )?;
        }
        Command::Projects { action } => match action.unwrap_or(ProjectAction::List) {
            ProjectAction::List => {
                for p in ctx.store.list_projects(true)? {
                    writeln!(
                        out,
                        "{}{}",
                        p.name,
                        if p.archived { "  (archived)" } else { "" }
                    )?;
                }
            }
            ProjectAction::Add { name } => {
                ctx.store.add_project(&name)?;
                writeln!(out, "Added {name}")?;
            }
            ProjectAction::Archive { name } => {
                ctx.store.archive_project(&name, true)?;
                writeln!(out, "Archived {name}")?;
            }
            ProjectAction::Unarchive { name } => {
                ctx.store.archive_project(&name, false)?;
                writeln!(out, "Unarchived {name}")?;
            }
            ProjectAction::Rename { old, new } => {
                ctx.store.rename_project(&old, &new)?;
                writeln!(out, "Renamed {old} → {new}")?;
            }
        },
        Command::Config {
            start,
            balance,
            target,
            vacation,
            hours,
        } => {
            let mut patch = ConfigPatch::default();
            if let Some(s) = &start {
                patch.start_date = Some(parse_date(s, today)?);
            }
            if let Some(b) = &balance {
                patch.initial_balance_minutes = Some(
                    b.parse::<Minutes>()
                        .map_err(|_| anyhow!("--balance '{b}': expected ±HH:MM, e.g. +12:30"))?
                        .0,
                );
            }
            if let Some(t) = &target {
                // Greater than zero is checked by `Config::validate`, which names the
                // field the config file uses, so the message matches a hand-edited file.
                patch.daily_target_minutes = Some(
                    t.parse::<Minutes>()
                        .map_err(|_| anyhow!("--target '{t}': expected HH:MM, e.g. 07:48"))?
                        .0,
                );
            }
            patch.vacation_days_per_year = vacation;
            if let Some(h) = &hours {
                patch.hours_format = Some(
                    HoursFormat::parse(h)
                        .ok_or_else(|| anyhow!("--hours '{h}': expected \"hm\" or \"decimal\""))?,
                );
            }
            if patch == ConfigPatch::default() {
                out.write_all(config_table(&ctx.config).as_bytes())?;
            } else {
                let cfg = Config::write_updates(&ctx.home, &patch)?;
                writeln!(out, "Updated config:")?;
                out.write_all(config_table(&cfg).as_bytes())?;
                writeln!(out, "Restart tk to apply in the TUI.")?;
            }
        }
        Command::Backup => {
            let path = write_backup(ctx, now)?;
            writeln!(out, "Backup written to {}", path.display())?;
        }
        Command::Export {
            format,
            from,
            to,
            output,
        } => {
            let rules = ctx.config.rules();
            let from = match from {
                Some(f) => parse_date(&f, today)?,
                None => rules.start_date,
            };
            let to = match to {
                Some(t) => parse_date(&t, today)?,
                None => today,
            };
            let entries = ctx.store.entries_in(from, to)?;
            // Both columns are data, not display: they stay ±HH:MM whatever
            // `hours_format` says.
            let nets = nets_by_day(&entries, &rules.tiers);
            let mut text = String::new();
            match format.as_str() {
                "csv" => {
                    text.push_str("date,start,end,project,comment,gross,net,break\n");
                    for (e, net) in entries.iter().zip(&nets) {
                        text.push_str(&format!(
                            "{},{},{},{},{},{},{},{}\n",
                            e.date,
                            e.start.format("%H:%M"),
                            e.end.format("%H:%M"),
                            csv_quote(&e.project),
                            csv_quote(&e.comment),
                            e.duration(),
                            net,
                            // What this entry paid towards its session's break:
                            // never negative, so it carries no sign.
                            (e.duration() - *net).hhmm()
                        ));
                    }
                }
                "json" => {
                    text.push('[');
                    for (i, (e, net)) in entries.iter().zip(&nets).enumerate() {
                        if i > 0 {
                            text.push(',');
                        }
                        text.push_str(&format!(
                            "{{\"date\":\"{}\",\"start\":\"{}\",\"end\":\"{}\",\"project\":{},\"comment\":{},\"gross_minutes\":{},\"net_minutes\":{},\"break_minutes\":{}}}",
                            e.date,
                            e.start.format("%H:%M"),
                            e.end.format("%H:%M"),
                            json_quote(&e.project),
                            json_quote(&e.comment),
                            e.duration().0,
                            net.0,
                            (e.duration() - *net).0
                        ));
                    }
                    text.push_str("]\n");
                }
                other => bail!("unknown format '{other}'; use csv or json"),
            }
            match output {
                Some(p) => {
                    std::fs::write(&p, text)?;
                    writeln!(out, "Wrote {}", p.display())?;
                }
                None => out.write_all(text.as_bytes())?,
            }
        }
        Command::Import { file, dry_run } => {
            let (report, msg) = import_file(ctx, &file, dry_run)?;
            writeln!(out, "{msg}")?;
            // The lines say which rows need a look; the count alone does not.
            for (line, why) in &report.skipped {
                writeln!(out, "  line {line}: {why}")?;
            }
        }
    }
    Ok(())
}

/// Read `path`, apply it, and return the report with the one-line summary that
/// both the CLI and the TUI show.
pub fn import_file(
    ctx: &Ctx,
    path: &Path,
    dry_run: bool,
) -> anyhow::Result<(ImportReport, String)> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let rows = parse_import(&text)?;
    let report = ctx.store.import_entries(&rows, dry_run)?;
    let mut msg = format!("imported {}", report.imported);
    if !report.skipped.is_empty() {
        msg.push_str(&format!(", skipped {}", report.skipped.len()));
    }
    if !report.new_projects.is_empty() {
        msg.push_str(&format!(", {} new projects", report.new_projects.len()));
    }
    if dry_run {
        msg.push_str(" (dry run, nothing written)");
    }
    Ok((report, msg))
}

/// The five settings `tk config` shows, one per line, label padded to 16.
fn config_table(c: &Config) -> String {
    let f = c.hours_format();
    [
        ("start_date", c.start_date.to_string()),
        (
            "initial_balance",
            Minutes(c.initial_balance_minutes).fmt_signed(f),
        ),
        (
            "daily_target",
            Minutes(c.daily_target_minutes).fmt_unsigned(f),
        ),
        ("vacation_days", c.vacation_days_per_year.to_string()),
        ("hours_format", c.hours_format.clone()),
    ]
    .iter()
    .map(|(k, v)| format!("{k:<16} {v}\n"))
    .collect()
}

fn csv_quote(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn json_quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, DEFAULT_TOML};
    use crate::store::Store;
    use chrono::NaiveTime;

    fn ctx() -> Ctx {
        Ctx {
            home: std::path::PathBuf::from("/nonexistent"),
            config: Config::from_toml(DEFAULT_TOML).unwrap(),
            store: Store::open_in_memory().unwrap(),
        }
    }

    fn dt(y: i32, m: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDateTime::new(
            NaiveDate::from_ymd_opt(y, m, d).unwrap(),
            NaiveTime::from_hms_opt(h, mi, 0).unwrap(),
        )
    }

    #[test]
    fn write_backup_twice_at_the_same_second_refreshes_the_one_file() {
        let dir = tempfile::tempdir().unwrap();
        let c = Ctx {
            home: dir.path().to_path_buf(),
            config: Config::from_toml(DEFAULT_TOML).unwrap(),
            store: Store::open_in_memory().unwrap(),
        };
        let now = dt(2026, 9, 17, 12, 0);
        let first = write_backup(&c, now).unwrap();
        let second = write_backup(&c, now).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            std::fs::read_dir(dir.path().join("backups"))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn status_line_keeps_counting_across_midnight() {
        let c = ctx();
        // Clocked in yesterday at 23:00, asked at 01:00 the next day: two hours, not zero.
        c.store
            .clock_in(
                NaiveDate::from_ymd_opt(2026, 9, 14).unwrap(),
                NaiveTime::from_hms_opt(23, 0, 0).unwrap(),
                "Alpha",
            )
            .unwrap();
        let line = status_line(&c, dt(2026, 9, 15, 1, 0)).unwrap();
        assert!(line.starts_with("⏱ Alpha 02:00 (in 23:00)"), "{line}");
        assert!(line.contains("today +02:00"), "{line}");
        assert!(line.contains("balance"), "{line}");
    }

    #[test]
    fn status_line_without_a_session_shows_net_and_balance() {
        let c = ctx();
        let line = status_line(&c, dt(2026, 9, 15, 10, 0)).unwrap();
        assert!(line.starts_with("not clocked in · today"), "{line}");
        assert!(line.contains("balance"), "{line}");
    }
}
