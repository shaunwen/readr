use std::process::Command;

use anyhow::{Result, bail};
use readr_core::config::NotifyChannel;

const SOUND: &str = "Glass";
const GROUP: &str = "readr";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    Herdr,
    TerminalNotifier,
    AppleScript { skipped: Vec<String> },
}

impl Delivery {
    pub fn describe(&self) -> String {
        match self {
            Self::Herdr => "herdr".into(),
            Self::TerminalNotifier => "terminal-notifier".into(),
            Self::AppleScript { skipped } if skipped.is_empty() => "osascript".into(),
            Self::AppleScript { skipped } => format!("osascript ({})", skipped.join("; ")),
        }
    }
}

pub fn notify(channel: NotifyChannel, title: &str, body: &str) -> Result<Delivery> {
    let mut skipped = Vec::new();
    if channel == NotifyChannel::Herdr {
        match herdr(title, body) {
            Ok(()) => return Ok(Delivery::Herdr),
            Err(reason) => skipped.push(format!("herdr: {reason}")),
        }
    }
    match terminal_notifier(title, body) {
        Ok(()) => return Ok(Delivery::TerminalNotifier),
        Err(reason) => skipped.push(format!("terminal-notifier: {reason}")),
    }
    apple_script(title, body)?;
    Ok(Delivery::AppleScript { skipped })
}

fn herdr(title: &str, body: &str) -> std::result::Result<(), String> {
    let output = Command::new("herdr")
        .args([
            "notification",
            "show",
            title,
            "--body",
            body,
            "--sound",
            "request",
        ])
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() && herdr_shown(&output.stdout) {
        return Ok(());
    }
    Err(failure_text(&output))
}

fn terminal_notifier(title: &str, body: &str) -> std::result::Result<(), String> {
    let output = Command::new("terminal-notifier")
        .args(terminal_notifier_args(title, body))
        .output()
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => "not installed".to_string(),
            _ => error.to_string(),
        })?;
    if output.status.success() {
        return Ok(());
    }
    Err(failure_text(&output))
}

fn terminal_notifier_args(title: &str, body: &str) -> Vec<String> {
    vec![
        "-title".into(),
        escape_leading(title),
        "-message".into(),
        escape_leading(body),
        "-sound".into(),
        SOUND.into(),
        "-group".into(),
        GROUP.into(),
    ]
}

fn escape_leading(text: &str) -> String {
    match text.chars().next() {
        Some('[' | '-' | '\\') => format!("\\{text}"),
        _ => text.to_string(),
    }
}

fn apple_script(title: &str, body: &str) -> Result<()> {
    let script = format!(
        "display notification {} with title {} sound name \"{SOUND}\"",
        applescript_string(body),
        applescript_string(title)
    );
    let output = Command::new("osascript").args(["-e", &script]).output()?;
    if !output.status.success() {
        bail!("notification failed: {}", failure_text(&output));
    }
    Ok(())
}

fn failure_text(output: &std::process::Output) -> String {
    [&output.stderr, &output.stdout]
        .iter()
        .map(|bytes| String::from_utf8_lossy(bytes).trim().to_string())
        .find(|text| !text.is_empty())
        .unwrap_or_else(|| format!("exit {}", output.status))
}

fn herdr_shown(stdout: &[u8]) -> bool {
    let text = String::from_utf8_lossy(stdout);
    text.contains("\"shown\":true")
}

fn applescript_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applescript_strings_escape_quotes_and_backslashes() {
        // given
        let text = r#"Read "Walden" \ p.9"#;

        // when
        let quoted = applescript_string(text);

        // then
        assert_eq!(quoted, r#""Read \"Walden\" \\ p.9""#);
    }

    #[test]
    fn rate_limited_herdr_notification_is_not_shown() {
        // given
        let stdout =
            br#"{"id":"cli:notification:show","result":{"reason":"rate_limited","shown":false}}"#;

        // when
        let shown = herdr_shown(stdout);

        // then
        assert!(!shown);
    }

    #[test]
    fn shown_herdr_notification_is_delivered() {
        // given
        let stdout = br#"{"id":"cli:notification:show","result":{"reason":"shown","shown":true}}"#;

        // when
        let shown = herdr_shown(stdout);

        // then
        assert!(shown);
    }

    #[test]
    fn terminal_notifier_gets_sound_and_replaces_the_previous_reminder() {
        // given
        let (title, body) = ("readr: 8 min to go today", "Walden p.9/300 · readr go");

        // when
        let args = terminal_notifier_args(title, body);

        // then
        assert_eq!(
            args,
            [
                "-title",
                "readr: 8 min to go today",
                "-message",
                "Walden p.9/300 · readr go",
                "-sound",
                "Glass",
                "-group",
                "readr"
            ]
        );
    }

    #[test]
    fn leading_brackets_and_dashes_are_escaped_for_terminal_notifier() {
        // given
        let texts = ["[draft] Walden", "-p.9", "Walden"];

        // when
        let escaped: Vec<String> = texts.iter().map(|t| escape_leading(t)).collect();

        // then
        assert_eq!(escaped, [r"\[draft] Walden", r"\-p.9", "Walden"]);
    }

    #[test]
    fn delivery_explains_why_it_fell_back() {
        // given
        let delivery = Delivery::AppleScript {
            skipped: vec!["terminal-notifier: not installed".into()],
        };

        // when
        let text = delivery.describe();

        // then
        assert_eq!(text, "osascript (terminal-notifier: not installed)");
    }
}
