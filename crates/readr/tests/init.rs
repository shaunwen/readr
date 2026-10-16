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
fn init_writes_the_default_config_once() {
    // given
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("cfg/readr/config.toml");

    // when
    command(dir.path())
        .arg("init")
        .assert()
        .success()
        .stdout(contains("wrote"));
    std::fs::write(&config, "daily_goal_minutes = 30\n").unwrap();
    let second = command(dir.path()).arg("init").assert();

    // then
    second.success().stdout(contains("config exists"));
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        "daily_goal_minutes = 30\n"
    );
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
