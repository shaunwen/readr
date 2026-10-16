use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use readr_core::config::Terminal;
use readr_core::db::Source;
use readr_core::model::Reader;

const RIO_BUNDLE: &str = "com.raphaelamorim.rio";

pub fn open(source: &Source, title: &str, terminal: Terminal) -> Result<()> {
    match source.reader {
        Reader::Bookokrat => open_bookokrat(Path::new(&source.locator), title, terminal),
        Reader::Preview => spawn(Command::new("open").args(["-a", "Preview", &source.locator])),
        Reader::AppleBooks => {
            spawn(Command::new("open").arg(format!("ibooks://assetid/{}", source.locator)))
        }
        Reader::Manual => bail!("{title} is only logged by hand; open it wherever you read it"),
    }
}

fn open_bookokrat(file: &Path, title: &str, terminal: Terminal) -> Result<()> {
    if !file.exists() {
        bail!("{} no longer exists", file.display());
    }
    let dir = file.parent().unwrap_or(Path::new("/"));
    match terminal {
        Terminal::Rio => spawn(
            Command::new("open")
                .args(["-n", "-b", RIO_BUNDLE, "--args", "-w"])
                .arg(dir)
                .args(["--title-placeholder", &format!("readr: {title}")])
                .args(["-e", "bookokrat"])
                .arg(file),
        ),
        Terminal::Herdr => {
            let output = Command::new("herdr")
                .args(["tab", "create", "--label", title])
                .output()
                .context("running herdr")?;
            if !output.status.success() {
                bail!(
                    "herdr tab create failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            let json = String::from_utf8_lossy(&output.stdout);
            let pane = root_pane_id(&json).context("herdr did not return a pane id")?;
            let command = format!(
                "cd {} && bookokrat {}",
                shell_quote(&dir.to_string_lossy()),
                shell_quote(&file.to_string_lossy())
            );
            spawn(Command::new("herdr").args(["pane", "run", &pane, &command]))
        }
    }
}

fn spawn(command: &mut Command) -> Result<()> {
    let status = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .with_context(|| format!("running {:?}", command.get_program()))?;
    if !status.success() {
        bail!("{:?} exited with {status}", command.get_program());
    }
    Ok(())
}

fn root_pane_id(json: &str) -> Option<String> {
    let after = json.split("\"root_pane\"").nth(1)?;
    let after = after.split("\"pane_id\"").nth(1)?;
    let value = after.split('"').nth(1)?;
    Some(value.to_string())
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn herdr_root_pane_is_read_from_tab_create_output() {
        // given
        let json = r#"{"id":"cli:tab:create","result":{"tab":{"tab_id":"wA:t3"},"root_pane":{"pane_id":"wA:p9","tab_id":"wA:t3"}}}"#;

        // when
        let pane = root_pane_id(json);

        // then
        assert_eq!(pane.as_deref(), Some("wA:p9"));
    }

    #[test]
    fn shell_quote_survives_apostrophes() {
        // given
        let path = "/Books/Man's Search for Meaning.epub";

        // when
        let quoted = shell_quote(path);

        // then
        assert_eq!(quoted, r"'/Books/Man'\''s Search for Meaning.epub'");
    }
}
