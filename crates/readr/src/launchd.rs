use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

pub const LABEL: &str = "io.github.shaunwen.readr";

pub struct Agent {
    pub plist: PathBuf,
    pub log: PathBuf,
    pub program: PathBuf,
    pub env: Vec<(String, String)>,
}

impl Agent {
    pub fn new(home: &Path, program: PathBuf, env: Vec<(String, String)>) -> Self {
        Self {
            plist: home.join(format!("Library/LaunchAgents/{LABEL}.plist")),
            log: home.join("Library/Logs/readr/daemon.log"),
            program,
            env,
        }
    }

    pub fn render(&self) -> String {
        let env: String = self
            .env
            .iter()
            .map(|(key, value)| {
                format!(
                    "        <key>{}</key>\n        <string>{}</string>\n",
                    escape(key),
                    escape(value)
                )
            })
            .collect();
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{program}</string>
        <string>daemon</string>
        <string>run</string>
    </array>
    <key>EnvironmentVariables</key>
    <dict>
{env}    </dict>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ThrottleInterval</key>
    <integer>30</integer>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
</dict>
</plist>
"#,
            program = escape(&self.program.to_string_lossy()),
            log = escape(&self.log.to_string_lossy()),
        )
    }

    pub fn install(&self) -> Result<()> {
        for dir in [self.plist.parent(), self.log.parent()]
            .into_iter()
            .flatten()
        {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(&self.plist, self.render())
            .with_context(|| format!("writing {}", self.plist.display()))?;
        let _ = launchctl(&["bootout", &service_target()]);
        launchctl(&["bootstrap", &domain_target(), &self.plist.to_string_lossy()])
    }

    pub fn uninstall(&self) -> Result<()> {
        let _ = launchctl(&["bootout", &service_target()]);
        if self.plist.exists() {
            std::fs::remove_file(&self.plist)
                .with_context(|| format!("removing {}", self.plist.display()))?;
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum State {
    Running { pid: u32 },
    Loaded { state: String },
    NotLoaded,
}

pub fn state() -> State {
    let Ok(output) = Command::new("launchctl")
        .args(["print", &service_target()])
        .output()
    else {
        return State::NotLoaded;
    };
    if !output.status.success() {
        return State::NotLoaded;
    }
    parse_state(&String::from_utf8_lossy(&output.stdout))
}

fn parse_state(print: &str) -> State {
    let field = |name: &str| {
        print.lines().find_map(|line| {
            let (key, value) = line.trim().split_once(" = ")?;
            (key == name).then(|| value.trim().to_string())
        })
    };
    match (
        field("state"),
        field("pid").and_then(|pid| pid.parse().ok()),
    ) {
        (Some(state), Some(pid)) if state == "running" => State::Running { pid },
        (Some(state), _) => State::Loaded { state },
        (None, _) => State::NotLoaded,
    }
}

fn uid() -> String {
    Command::new("id")
        .arg("-u")
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|_| "501".into())
}

fn domain_target() -> String {
    format!("gui/{}", uid())
}

fn service_target() -> String {
    format!("gui/{}/{LABEL}", uid())
}

fn launchctl(args: &[&str]) -> Result<()> {
    let output = Command::new("launchctl").args(args).output()?;
    if !output.status.success() {
        bail!(
            "launchctl {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_runs_the_daemon_with_logs_and_environment() {
        // given
        let agent = Agent::new(
            Path::new("/Users/reader"),
            PathBuf::from("/Users/reader/.local/bin/readr"),
            vec![("PATH".into(), "/opt/homebrew/bin:/usr/bin:/bin".into())],
        );

        // when
        let plist = agent.render();

        // then
        assert!(plist.contains("<string>/Users/reader/.local/bin/readr</string>"));
        assert!(plist.contains("<string>daemon</string>\n        <string>run</string>"));
        assert!(
            plist.contains(
                "<key>PATH</key>\n        <string>/opt/homebrew/bin:/usr/bin:/bin</string>"
            )
        );
        assert!(plist.contains("<string>/Users/reader/Library/Logs/readr/daemon.log</string>"));
        assert_eq!(
            agent.plist,
            PathBuf::from("/Users/reader/Library/LaunchAgents/io.github.shaunwen.readr.plist")
        );
    }

    #[test]
    fn plist_is_valid_for_launchd() {
        // given
        let dir = tempfile::tempdir().unwrap();
        let agent = Agent::new(
            dir.path(),
            PathBuf::from("/opt/readr & co/readr"),
            vec![("XDG_DATA_HOME".into(), "/tmp/data".into())],
        );
        let path = dir.path().join("agent.plist");
        std::fs::write(&path, agent.render()).unwrap();

        // when
        let lint = Command::new("plutil")
            .arg("-lint")
            .arg(&path)
            .output()
            .unwrap();

        // then
        assert!(
            lint.status.success(),
            "{}",
            String::from_utf8_lossy(&lint.stdout)
        );
    }

    #[test]
    fn running_service_reports_its_pid() {
        // given
        let print = "gui/501/io.github.shaunwen.readr = {\n\tactive count = 1\n\tstate = running\n\tpid = 4242\n}";

        // when
        let state = parse_state(print);

        // then
        assert_eq!(state, State::Running { pid: 4242 });
    }

    #[test]
    fn stopped_service_reports_its_state() {
        // given
        let print = "gui/501/io.github.shaunwen.readr = {\n\tstate = not running\n}";

        // when
        let state = parse_state(print);

        // then
        assert_eq!(
            state,
            State::Loaded {
                state: "not running".into()
            }
        );
    }
}
