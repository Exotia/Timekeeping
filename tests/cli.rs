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
    tk(home)
        .arg("in")
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
