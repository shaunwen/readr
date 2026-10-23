use assert_cmd::Command;
use predicates::str::contains;

fn command(dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("readr").unwrap();
    cmd.env("HOME", dir)
        .env("XDG_CONFIG_HOME", dir.join("cfg"))
        .env("XDG_DATA_HOME", dir.join("data"));
    cmd
}

#[test]
fn init_starts_with_the_default_goals() {
    // given
    let dir = tempfile::tempdir().unwrap();

    // when
    let assert = command(dir.path()).arg("init").assert();

    // then
    assert
        .success()
        .stdout(contains("settings: 4 goals, 20 min a day"));
}

#[test]
fn config_toml_is_imported_once_and_then_ignored() {
    // given
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("cfg/readr/config.toml");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, "daily_goal_minutes = 30\n").unwrap();
    command(dir.path())
        .args(["status", "--short"])
        .assert()
        .success();

    // when
    std::fs::write(&config, "daily_goal_minutes = 45\n").unwrap();
    let assert = command(dir.path()).args(["status", "--short"]).assert();

    // then
    assert.success().stdout("0/30m\n");
}

#[test]
fn config_import_loads_the_file_again() {
    // given
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("cfg/readr/config.toml");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, "daily_goal_minutes = 30\n").unwrap();
    command(dir.path())
        .args(["status", "--short"])
        .assert()
        .success();
    std::fs::write(&config, "daily_goal_minutes = 45\n").unwrap();

    // when
    command(dir.path())
        .args(["config", "import"])
        .assert()
        .success();

    // then
    command(dir.path())
        .args(["status", "--short"])
        .assert()
        .success()
        .stdout("0/45m\n");
}

#[test]
fn config_set_changes_one_setting() {
    // given
    let dir = tempfile::tempdir().unwrap();

    // when
    let set = command(dir.path())
        .args(["config", "set", "daily_goal_minutes", "25"])
        .assert();

    // then
    set.success().stdout("Daily goal (min) = 25\n");
    command(dir.path())
        .args(["status", "--short"])
        .assert()
        .stdout("0/25m\n");
}

#[test]
fn config_set_rejects_bad_values() {
    // given
    let dir = tempfile::tempdir().unwrap();

    // when
    let assert = command(dir.path())
        .args(["config", "set", "nightly.at", "9pm"])
        .assert();

    // then
    assert.failure().stderr(contains("time like 21:00"));
}

#[test]
fn config_shows_settings_as_toml() {
    // given
    let dir = tempfile::tempdir().unwrap();

    // when
    let assert = command(dir.path()).arg("config").assert();

    // then
    assert
        .success()
        .stdout(contains("daily_goal_minutes = 20"))
        .stdout(contains("[[goal]]"));
}

#[test]
fn custom_daily_goal_shows_in_status() {
    // given
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("cfg/readr")).unwrap();
    std::fs::write(
        dir.path().join("cfg/readr/config.toml"),
        "daily_goal_minutes = 30\n",
    )
    .unwrap();

    // when
    let assert = command(dir.path()).args(["status", "--short"]).assert();

    // then
    assert.success().stdout("0/30m\n");
}
