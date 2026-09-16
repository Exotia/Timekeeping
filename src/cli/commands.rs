use std::io::Write;

use anyhow::{Context as _, anyhow, bail};
use chrono::{Days, Local, NaiveDate, NaiveDateTime, Timelike};

use super::{Command, Ctx, ProjectAction};
use crate::config::{Config, ConfigPatch};
use crate::core::{
    DayKind, Minutes, TodayCtx, day_stats, parse_date, parse_time_range, provisional_net_with,
    running_balance, running_minutes,
};

pub fn now_local() -> NaiveDateTime {
    Local::now().naive_local()
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
    let rules = ctx.config.rules();
    let session = ctx.store.session()?;
    let balance = balance_as_of(ctx, today, session.is_some())?;
    let entries = ctx.store.entries_on(today)?;
    Ok(match session {
        Some(s) => {
            // The session carries its own date, so a clock-in from yesterday keeps
            // counting instead of wrapping back to 00:00 at midnight.
            let running = running_minutes(NaiveDateTime::new(s.date, s.start), now);
            let net = provisional_net_with(&entries, running, &rules);
            // A session recorded before `tk` stored the project simply has none to name.
            let project = s
                .project
                .as_deref()
                .map(|p| format!("{p} "))
                .unwrap_or_default();
            format!(
                "⏱ {}{} (in {}) · today {} · balance {}",
                project,
                running.hhmm(),
                s.start.format("%H:%M"),
                net,
                balance
            )
        }
        None => {
            let net: Minutes = {
                let cal = ctx.config.calendar();
                let d = ctx.store.days_in(today, today, &cal)?.remove(0);
                day_stats(
                    &d,
                    &rules,
                    &cal,
                    &TodayCtx {
                        today,
                        clocked_in: false,
                    },
                )
                .net
            };
            format!("not clocked in · today {net} · balance {balance}")
        }
    })
}

pub fn run(cmd: Command, ctx: &Ctx, out: &mut dyn Write) -> anyhow::Result<()> {
    let now = now_local();
    let today = now.date();
    match cmd {
        Command::In { force, project } => {
            if force {
                ctx.store.clear_session()?;
            }
            // A session that is already open is the more useful complaint, so it comes
            // before the one about a missing project. The store checks it again, under
            // its own lock, so a concurrent `tk in` is still refused.
            if let Some(s) = ctx.store.session()? {
                bail!(
                    "already clocked in since {} {}",
                    s.date,
                    s.start.format("%H:%M")
                );
            }
            let t = now.time().with_second(0).unwrap_or(now.time());
            let project = match project.or(ctx.store.last_used_project()?) {
                Some(p) => p,
                None => bail!("no project yet; pass --project NAME"),
            };
            ctx.store.clock_in(today, t, &project)?;
            writeln!(out, "Clocked in on {project} at {}", t.format("%H:%M"))?;
        }
        Command::Switch { project, comment } => {
            let (e, s) =
                ctx.store
                    .switch_project(now.time(), &project, comment.as_deref().unwrap_or(""))?;
            writeln!(
                out,
                "Booked {}–{} {} ({}) · now on {} since {}",
                e.start.format("%H:%M"),
                e.end.format("%H:%M"),
                e.project,
                e.duration(),
                s.project.as_deref().unwrap_or(&project),
                s.start.format("%H:%M")
            )?;
        }
        Command::Out { project, comment } => {
            // One transaction in the store: the entry and the cleared session, or neither.
            let e = ctx.store.clock_out_with(
                now.time(),
                project.as_deref(),
                comment.as_deref().unwrap_or(""),
            )?;
            let stats = day_stats(
                &ctx.store
                    .days_in(e.date, e.date, &ctx.config.calendar())?
                    .remove(0),
                &ctx.config.rules(),
                &ctx.config.calendar(),
                &TodayCtx {
                    today,
                    clocked_in: false,
                },
            );
            writeln!(
                out,
                "Recorded {}–{} {} ({}) · day net {} · balance {}",
                e.start.format("%H:%M"),
                e.end.format("%H:%M"),
                e.project,
                e.duration(),
                stats.net,
                balance_as_of(ctx, today, false)?
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
            let st = day_stats(
                &day,
                &ctx.config.rules(),
                &ctx.config.calendar(),
                &TodayCtx {
                    today,
                    clocked_in: false,
                },
            );
            writeln!(
                out,
                "Added {} {}–{} {} · gross {} · day net {}",
                date,
                entry.start.format("%H:%M"),
                entry.end.format("%H:%M"),
                entry.project,
                entry.duration(),
                st.net
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
            let dir = ctx.home.join("backups");
            std::fs::create_dir_all(&dir)?;
            let path = dir.join(format!("tk-{}.db", now.format("%Y%m%d-%H%M%S")));
            ctx.store.backup_to(&path)?;
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
            let mut text = String::new();
            match format.as_str() {
                "csv" => {
                    text.push_str("date,start,end,project,comment,gross\n");
                    for e in &entries {
                        text.push_str(&format!(
                            "{},{},{},{},{},{}\n",
                            e.date,
                            e.start.format("%H:%M"),
                            e.end.format("%H:%M"),
                            csv_quote(&e.project),
                            csv_quote(&e.comment),
                            e.duration()
                        ));
                    }
                }
                "json" => {
                    text.push('[');
                    for (i, e) in entries.iter().enumerate() {
                        if i > 0 {
                            text.push(',');
                        }
                        text.push_str(&format!(
                            "{{\"date\":\"{}\",\"start\":\"{}\",\"end\":\"{}\",\"project\":{},\"comment\":{},\"gross_minutes\":{}}}",
                            e.date,
                            e.start.format("%H:%M"),
                            e.end.format("%H:%M"),
                            json_quote(&e.project),
                            json_quote(&e.comment),
                            e.duration().0
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
    }
    Ok(())
}

/// The four settings `tk config` shows, one per line, label padded to 16.
fn config_table(c: &Config) -> String {
    [
        ("start_date", c.start_date.to_string()),
        (
            "initial_balance",
            Minutes(c.initial_balance_minutes).to_string(),
        ),
        ("daily_target", Minutes(c.daily_target_minutes).hhmm()),
        ("vacation_days", c.vacation_days_per_year.to_string()),
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
