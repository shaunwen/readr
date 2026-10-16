use std::process::Command;

use anyhow::{Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Channel {
    Herdr,
    MacOs { herdr_error: String },
}

pub fn notify(title: &str, body: &str) -> Result<Channel> {
    let herdr_error = match Command::new("herdr")
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
    {
        Ok(output) if output.status.success() && herdr_shown(&output.stdout) => {
            return Ok(Channel::Herdr);
        }
        Ok(output) => [output.stderr, output.stdout]
            .iter()
            .map(|bytes| String::from_utf8_lossy(bytes).trim().to_string())
            .find(|text| !text.is_empty())
            .unwrap_or_else(|| format!("exit {}", output.status)),
        Err(error) => error.to_string(),
    };
    let script = format!(
        "display notification {} with title {} sound name \"Glass\"",
        applescript_string(body),
        applescript_string(title)
    );
    let mac = Command::new("osascript").args(["-e", &script]).output()?;
    if !mac.status.success() {
        bail!(
            "notification failed: herdr: {herdr_error}; macOS: {}",
            String::from_utf8_lossy(&mac.stderr).trim()
        );
    }
    Ok(Channel::MacOs { herdr_error })
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
}
