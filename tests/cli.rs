use assert_cmd::Command;
use predicates::prelude::*;

fn tk(home: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("tk").unwrap();
    c.env("TK_HOME", home);
    c
}

#[test]
fn first_run_creates_home_and_config() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("h");
    tk(&home)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("not clocked in"));
    assert!(home.join("config.toml").exists());
    assert!(home.join("tk.db").exists());
}

#[test]
fn add_day_and_status_and_export() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    tk(home)
        .args([
            "add",
            "2026-09-14",
            "0900-1530",
            "-p",
            "Alpha",
            "-m",
            "note",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("+05:42"));
    tk(home)
        .args(["add", "2026-09-14", "1000-1100", "-p", "Alpha"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("overlap"));
    tk(home)
        .args(["add", "2026-09-14", "0900-0900", "-p", "Alpha"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("differ"));
    tk(home)
        .args(["day", "2026-09-15", "vacation", "--to", "2026-09-16"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 day"));
    tk(home)
        .args(["day", "2026-09-14", "sick"])
        .assert()
        .failure();
    tk(home)
        .args(["projects"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Alpha"));
    tk(home)
        .args(["export", "--format", "csv"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "date,start,end,project,comment,gross,net,break",
        ))
        // 6:30 gross in one session: 48 minutes off it leaves 5:42 net, and the
        // `break` column says where the difference went.
        .stdout(predicate::str::contains(
            "2026-09-14,09:00,15:30,Alpha,note,+06:30,+05:42,00:48",
        ));
    tk(home)
        .args(["export", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"project\":\"Alpha\""))
        .stdout(predicate::str::contains(
            "\"gross_minutes\":390,\"net_minutes\":342,\"break_minutes\":48",
        ));
}

/// A pause splits the day: each seamless session is charged on its own length.
#[test]
fn the_break_deduction_is_charged_per_session() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    tk(home)
        .args(["add", "today", "0800-1200", "-p", "Alpha"])
        .assert()
        .success()
        .stdout(predicate::str::contains("day net +03:42")); // 4h gross − 18
    // Back at 12:45: two sessions of 4:00 and 4:15, each over three hours, so
    // 8:15 gross loses 18 + 18.
    tk(home)
        .args(["add", "today", "1245-1700", "-p", "Alpha"])
        .assert()
        .success()
        .stdout(predicate::str::contains("day net +07:39"));
    tk(home)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("today +07:39"));
    // The same hours worked straight through lose the full 48 minutes.
    let dir2 = tempfile::tempdir().unwrap();
    let home2 = dir2.path();
    tk(home2)
        .args(["add", "today", "0800-1200", "-p", "Alpha"])
        .assert()
        .success();
    tk(home2)
        .args(["add", "today", "1200-1700", "-p", "Alpha"])
        .assert()
        .success()
        .stdout(predicate::str::contains("day net +08:12")); // 9h gross − 48
    tk(home2)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("today +08:12"));
}

/// A booked entry reports the net it earned, not just its gross: the session's
/// break deduction is shared out over the entries of that session.
#[test]
fn a_booked_entry_reports_its_own_net() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    // On its own, the morning is a four-hour session losing 18 minutes.
    tk(home)
        .args(["add", "today", "0800-1200", "-p", "Alpha"])
        .assert()
        .success()
        .stdout(predicate::str::contains("gross +04:00"))
        .stdout(predicate::str::contains("net +03:42"))
        .stdout(predicate::str::contains("day net +03:42"));
    // Straight on at noon: one nine-hour session losing 48 minutes, and with no
    // share assigned they all fall on the project worked last.
    tk(home)
        .args(["add", "today", "1200-1700", "-p", "Beta"])
        .assert()
        .success()
        .stdout(predicate::str::contains("gross +05:00"))
        .stdout(predicate::str::contains("net +04:12"))
        .stdout(predicate::str::contains("day net +08:12"));
    // And the morning's share is re-read from the session it now belongs to:
    // the 18 minutes it paid on its own are given back.
    let shown = tk(home)
        .args(["export", "--format", "csv"])
        .assert()
        .success();
    let csv = String::from_utf8(shown.get_output().stdout.clone()).unwrap();
    assert!(csv.contains("Alpha,,+04:00,+04:00,00:00"), "{csv}");
    assert!(csv.contains("Beta,,+05:00,+04:12,00:48"), "{csv}");
}

#[test]
fn clock_in_and_out() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    // A fresh home has no project to fall back on, so the first clock-in names one.
    tk(home)
        .arg("in")
        .assert()
        .failure()
        .stderr(predicate::str::contains("no project"));
    tk(home)
        .args(["in", "-p", "Beta"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Clocked in"));
    tk(home)
        .arg("in")
        .assert()
        .failure()
        .stderr(predicate::str::contains("already clocked in"));
    tk(home)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("⏱"));
    tk(home)
        .args(["add", "yesterday", "0900-1000", "-p", "Beta"])
        .assert()
        .success();
    tk(home)
        .arg("out")
        .assert()
        .success()
        .stdout(predicate::str::contains("Beta")) // the project the session was opened on
        .stdout(predicate::str::contains("· net "));
    tk(home)
        .arg("out")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not clocked in"));
}

/// A break books the work so far and keeps the clock on the project: coming back
/// is a `tk in`, and the two halves are two sessions, each charged on its own.
#[test]
fn break_books_the_work_so_far_and_resumes_on_the_same_project() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    tk(home).args(["in", "-p", "Alpha"]).assert().success();
    tk(home)
        .args(["break", "-m", "lunch"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Booked "))
        .stdout(predicate::str::contains("Alpha"))
        .stdout(predicate::str::contains("· on break since "));
    let shown = tk(home).arg("status").assert().success();
    let line = String::from_utf8(shown.get_output().stdout.clone()).unwrap();
    assert!(line.contains("on break"), "{line}");
    assert!(line.contains("Alpha"), "{line}");
    assert!(line.contains("balance"), "{line}");
    // A switch is not the way back, and it says which key is.
    tk(home)
        .args(["switch", "-p", "Beta"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("on break"));
    // Coming back without naming a project comes back on the remembered one.
    tk(home)
        .arg("in")
        .assert()
        .success()
        .stdout(predicate::str::contains("Resumed Alpha at "));
    tk(home)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("\u{23f1} Alpha"));
    tk(home).arg("out").assert().success();
    // Both halves are on the books, the morning booked by the break itself.
    let shown = tk(home)
        .args(["export", "--format", "csv"])
        .assert()
        .success();
    let csv = String::from_utf8(shown.get_output().stdout.clone()).unwrap();
    let rows: Vec<&str> = csv.lines().skip(1).filter(|l| !l.is_empty()).collect();
    assert_eq!(rows.len(), 2, "{csv}");
    assert!(rows[0].contains(",Alpha,lunch,"), "{csv}");
    assert!(rows[1].contains(",Alpha,"), "{csv}");
}

#[test]
fn a_break_can_be_resumed_on_another_project_or_ended_outright() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    tk(home).args(["in", "-p", "Alpha"]).assert().success();
    tk(home).arg("break").assert().success();
    // A second break has nothing left to book.
    tk(home)
        .arg("break")
        .assert()
        .failure()
        .stderr(predicate::str::contains("already on break"));
    tk(home)
        .args(["in", "-p", "Beta"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Resumed Beta at "));
    tk(home).arg("out").assert().success();
    let shown = tk(home)
        .args(["export", "--format", "csv"])
        .assert()
        .success();
    let csv = String::from_utf8(shown.get_output().stdout.clone()).unwrap();
    let rows: Vec<&str> = csv.lines().skip(1).filter(|l| !l.is_empty()).collect();
    assert_eq!(rows.len(), 2, "{csv}");
    assert!(rows[0].contains(",Alpha,"), "{csv}");
    assert!(rows[1].contains(",Beta,"), "{csv}");

    // Clocking out on a break ends it and books nothing more. A home of its own,
    // because a clock-in a minute after this one would land inside the entries
    // above — everything here happens in the same wall-clock minute.
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    tk(home).args(["in", "-p", "Alpha"]).assert().success();
    tk(home).arg("break").assert().success();
    tk(home)
        .arg("out")
        .assert()
        .success()
        .stdout(predicate::str::contains("Break ended after "))
        .stdout(predicate::str::contains("nothing to book"));
    tk(home)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("not clocked in"));
    // Nothing to take a break from once the clock is off.
    tk(home)
        .arg("break")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not clocked in"));
}

#[test]
fn clock_in_switch_and_out_are_project_aware() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    // Nothing worked yet, so there is no project to fall back on.
    tk(home)
        .arg("in")
        .assert()
        .failure()
        .stderr(predicate::str::contains("no project"));
    tk(home)
        .args(["in", "-p", "Alpha"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Clocked in on Alpha"));
    tk(home)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("\u{23f1} Alpha"));
    // Switching books the running session and opens the next one on the new project.
    let switched = tk(home)
        .args(["switch", "-p", "Beta", "-m", "standup"])
        .assert()
        .success();
    let line = String::from_utf8(switched.get_output().stdout.clone()).unwrap();
    assert!(line.contains("Alpha"), "the booked project: {line}");
    assert!(line.contains("· net "), "the entry's net: {line}");
    assert!(line.contains("now on Beta"), "the new session: {line}");
    // Already on Beta: refused rather than booking a second entry.
    tk(home)
        .args(["switch", "-p", "Beta"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already on"));
    tk(home)
        .arg("out")
        .assert()
        .success()
        .stdout(predicate::str::contains("Beta"));
    // Both entries are on the books, in order and without overlapping.
    let shown = tk(home)
        .args(["export", "--format", "csv"])
        .assert()
        .success();
    let csv = String::from_utf8(shown.get_output().stdout.clone()).unwrap();
    let rows: Vec<&str> = csv.lines().skip(1).filter(|l| !l.is_empty()).collect();
    assert_eq!(rows.len(), 2, "{csv}");
    assert!(rows[0].contains(",Alpha,"), "{csv}");
    assert!(rows[1].contains(",Beta,"), "{csv}");
}

#[test]
fn several_switches_inside_one_minute_each_book_their_own_minute() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    // Every switch opens the next session a minute ahead of the wall clock, so a run of
    // them inside one minute must still book one minute each — no overlap, no error.
    tk(home).args(["in", "-p", "Alpha"]).assert().success();
    tk(home).args(["switch", "-p", "Beta"]).assert().success();
    tk(home)
        .args(["switch", "-p", "Gamma"])
        .assert()
        .success()
        .stdout(predicate::str::contains("now on Gamma"));
    tk(home)
        .arg("out")
        .assert()
        .success()
        .stdout(predicate::str::contains("Gamma"));
    let shown = tk(home)
        .args(["export", "--format", "csv"])
        .assert()
        .success();
    let csv = String::from_utf8(shown.get_output().stdout.clone()).unwrap();
    let rows: Vec<&str> = csv.lines().skip(1).filter(|l| !l.is_empty()).collect();
    assert_eq!(rows.len(), 3, "{csv}");
    for (row, project) in rows.iter().zip(["Alpha", "Beta", "Gamma"]) {
        assert!(row.contains(&format!(",{project},")), "{csv}");
    }
}

#[test]
fn backup_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    tk(dir.path())
        .arg("backup")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"));
}

#[test]
fn bad_config_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        "start_date = \"2026-01-01\"\ndaily_target_minutes = 0\n",
    )
    .unwrap();
    tk(dir.path())
        .arg("status")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("daily_target_minutes"));
}

#[test]
fn config_shows_and_sets_values() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    // No flags: the current table, from the freshly written default file.
    tk(home)
        .arg("config")
        .assert()
        .success()
        .stdout(predicate::str::contains("daily_target     07:48"));
    tk(home)
        .args([
            "config",
            "--start",
            "2026-09-15",
            "--balance",
            "+12:30",
            "--target",
            "8:00",
            "--vacation",
            "28",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated config:"))
        .stdout(predicate::str::contains("Restart tk to apply in the TUI."));
    let shown = tk(home).arg("config").assert().success();
    let out = String::from_utf8(shown.get_output().stdout.clone()).unwrap();
    assert!(out.contains("start_date       2026-09-15"), "{out}");
    assert!(out.contains("initial_balance  +12:30"), "{out}");
    assert!(out.contains("daily_target     08:00"), "{out}");
    assert!(out.contains("vacation_days    28"), "{out}");
    // An invalid value is refused by name, and the file keeps the old value.
    tk(home)
        .args(["config", "--target", "0:00"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("daily_target_minutes"));
    tk(home)
        .arg("config")
        .assert()
        .success()
        .stdout(predicate::str::contains("daily_target     08:00"));
}

/// `hours_format` decides how every printed duration is spelled — except the
/// export, which stays `±HH:MM` so the files keep their shape.
#[test]
fn config_hours_switches_the_printed_durations() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    // The default is h:mm, and the table shows it as a fifth row.
    tk(home)
        .arg("config")
        .assert()
        .success()
        .stdout(predicate::str::contains("hours_format     hm"));
    tk(home)
        .args(["config", "--hours", "decimal"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hours_format     decimal"))
        .stdout(predicate::str::contains("daily_target     7.80h"))
        .stdout(predicate::str::contains("initial_balance  +0.00h"));
    tk(home)
        .args(["add", "2026-09-14", "0900-1530", "-p", "Alpha"])
        .assert()
        .success()
        .stdout(predicate::str::contains("gross +6.50h"))
        .stdout(predicate::str::contains("net +5.70h"));
    tk(home)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("today +0.00h"));
    // Exported hours are data, not display: both columns stay ±HH:MM.
    tk(home)
        .args(["export", "--format", "csv"])
        .assert()
        .success()
        .stdout(predicate::str::contains("+06:30,+05:42"));
    // Back to h:mm.
    tk(home)
        .args(["config", "--hours", "hm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hours_format     hm"));
    tk(home)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("today +00:00"));
    // An unknown spelling is refused and the file keeps its value.
    tk(home)
        .args(["config", "--hours", "industrial"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("decimal"));
    tk(home)
        .arg("config")
        .assert()
        .success()
        .stdout(predicate::str::contains("hours_format     hm"));
}

#[test]
fn config_start_today_moves_the_balance() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    // With the start date on today and nothing worked yet, the balance is exactly
    // the carried-over one.
    tk(home)
        .args(["config", "--start", "today", "--balance", "+12:30"])
        .assert()
        .success();
    tk(home)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("balance +12:30"));
}

#[test]
fn config_accepts_negative_values() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    // A negative balance and a date offset into the past both start with `-`, which
    // clap would otherwise read as the start of another flag.
    tk(home)
        .args(["config", "--start", "-1", "--balance", "-2:30"])
        .assert()
        .success();
    let yesterday = (chrono::Local::now().date_naive() - chrono::Days::new(1)).to_string();
    tk(home)
        .arg("config")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "start_date       {yesterday}"
        )))
        .stdout(predicate::str::contains("initial_balance  -02:30"));
}

#[test]
fn export_then_import_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let csv = dir.path().join("out.csv");
    tk(home)
        .args([
            "add",
            "2026-09-14",
            "0900-1530",
            "-p",
            "Alpha",
            "-m",
            "lunch, then review",
        ])
        .assert()
        .success();
    tk(home)
        .args(["export", "-o", csv.to_str().unwrap()])
        .assert()
        .success();

    // A fresh home: the same rows must land from the file alone.
    let home2 = dir.path().join("h2");
    tk(&home2)
        .args(["import", csv.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("imported 1"));
    tk(&home2)
        .args(["export"])
        .assert()
        .success()
        .stdout(predicate::str::contains("lunch, then review"));
}

#[test]
fn importing_the_same_file_twice_skips_everything() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let csv = dir.path().join("out.csv");
    std::fs::write(
        &csv,
        "date,start,end,project,comment,gross,net,break\n\
         2026-09-14,09:00,15:30,Alpha,note,06:30,05:42,00:48\n",
    )
    .unwrap();
    tk(home)
        .args(["import", csv.to_str().unwrap()])
        .assert()
        .success();
    tk(home)
        .args(["import", csv.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("skipped 1"));
}

#[test]
fn dry_run_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let csv = dir.path().join("out.csv");
    std::fs::write(
        &csv,
        "date,start,end,project,comment,gross,net,break\n\
         2026-09-14,09:00,15:30,Alpha,note,06:30,05:42,00:48\n",
    )
    .unwrap();
    tk(home)
        .args(["import", csv.to_str().unwrap(), "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("imported 1"));
    tk(home)
        .args(["export"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Alpha").not());
}

// Review Focus 4: nothing to do is not a failure.
#[test]
fn an_empty_file_imports_nothing_without_erroring() {
    let dir = tempfile::tempdir().unwrap();
    let csv = dir.path().join("empty.csv");
    std::fs::write(&csv, "").unwrap();
    tk(dir.path())
        .args(["import", csv.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("imported 0"));
}

// Review Focus 5, CLI half.
#[test]
fn a_missing_file_fails_with_its_path() {
    let dir = tempfile::tempdir().unwrap();
    tk(dir.path())
        .args(["import", "/nope/missing.csv"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("missing.csv"));
}
