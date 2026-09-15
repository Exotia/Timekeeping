use std::io::Write;

use anyhow::{Context as _, anyhow, bail};
use chrono::{Days, Local, NaiveDate, NaiveDateTime, TimeDelta, Timelike};

use super::{Command, Ctx, ProjectAction};
use crate::core::{
    DayKind, Minutes, TodayCtx, day_stats, parse_date, parse_time_range, provisional_net,
    running_balance,
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
            let running = Minutes(((now.time() - s.start).num_minutes()).max(0) as i32);
            let net = provisional_net(&entries, s.start, now.time(), &rules);
            format!(
                "⏱ {} (in {}) · today {} · balance {}",
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
        Command::In { force } => {
            if force {
                ctx.store.clear_session()?;
            }
            let t = now.time().with_second(0).unwrap_or(now.time());
            ctx.store.clock_in(today, t)?;
            writeln!(out, "Clocked in at {}", t.format("%H:%M"))?;
        }
        Command::Out { project, comment } => {
            let s = ctx
                .store
                .session()?
                .ok_or_else(|| anyhow!("not clocked in"))?;
            let project = match project.or(ctx.store.last_used_project()?) {
                Some(p) => p,
                None => bail!("no project given and none used before; pass --project NAME"),
            };
            let mut end = now.time().with_second(0).unwrap_or(now.time());
            if end == s.start {
                // A zero-length clock-out would look like a 24h shift once end <= start.
                // Record a 1-minute entry instead.
                end = end.overflowing_add_signed(TimeDelta::minutes(1)).0;
            }
            let e = ctx.store.add_entry(
                s.date,
                s.start,
                end,
                &project,
                comment.as_deref().unwrap_or(""),
            )?;
            ctx.store.clear_session()?;
            let stats = day_stats(
                &ctx.store
                    .days_in(s.date, s.date, &ctx.config.calendar())?
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
