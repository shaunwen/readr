use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};
use readr_core::config::Email;

const KEYCHAIN_SERVICE: &str = "readr-smtp";

pub fn send(email: &Email, subject: &str, body: &str) -> Result<()> {
    let password = password(&email.smtp_user)?;
    let message = Message::builder()
        .from(
            email
                .from
                .parse::<Mailbox>()
                .context("nightly.email_from")?,
        )
        .to(email.to.parse::<Mailbox>().context("nightly.email_to")?)
        .subject(subject)
        .body(body.to_string())?;
    let transport = SmtpTransport::starttls_relay(&email.smtp_host)?
        .port(email.smtp_port)
        .credentials(Credentials::new(email.smtp_user.clone(), password))
        .build();
    transport
        .send(&message)
        .with_context(|| format!("sending through {}:{}", email.smtp_host, email.smtp_port))?;
    Ok(())
}

pub fn set_password(account: &str) -> Result<()> {
    let status = Command::new("security")
        .args([
            "add-generic-password",
            "-U",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            account,
            "-w",
        ])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;
    if !status.success() {
        bail!("saving the SMTP password to the Keychain failed");
    }
    Ok(())
}

fn password(account: &str) -> Result<String> {
    let output = Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            account,
            "-w",
        ])
        .output()?;
    if !output.status.success() {
        bail!("no SMTP password in the Keychain for {account}; run `readr email set-password`");
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end_matches('\n')
        .to_string())
}
