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
            "2026-09-14,09:00,15:30,Alpha,note",
        ));
    tk(home)
        .args(["export", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"project\":\"Alpha\""));
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
        .stdout(predicate::str::contains("Beta")); // last-used project
    tk(home)
        .arg("out")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not clocked in"));
}

#[test]
fn backup_writes_file() {
    let dir = tempfile::tempdir().unwrap();
    tk(dir.path())
        .arg("backup")
        .assert()
        .success()
        .stdout(predicate::str::contains("backups/tk-"));
    assert_eq!(
        std::fs::read_dir(dir.path().join("backups"))
            .unwrap()
            .count(),
        1
    );
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
