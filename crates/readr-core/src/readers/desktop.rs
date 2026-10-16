use std::path::{Path, PathBuf};
use std::process::Command;

use jiff::SignedDuration;

pub const PREVIEW_BUNDLE: &str = "com.apple.Preview";
pub const BOOKS_BUNDLE: &str = "com.apple.iBooksX";

pub trait Desktop {
    fn frontmost_bundle_id(&self) -> Option<String>;
    fn idle(&self) -> Option<SignedDuration>;
    fn process_running(&self, name: &str) -> bool;
    fn preview_front_document(&self) -> Option<PathBuf>;
    fn page_count(&self, path: &Path) -> Option<u32>;
    fn document_title(&self, path: &Path) -> Option<String>;
}

pub struct MacDesktop;

impl Desktop for MacDesktop {
    fn frontmost_bundle_id(&self) -> Option<String> {
        let asn = run("lsappinfo", &["front"])?;
        let info = run("lsappinfo", &["info", "-only", "bundleid", asn.trim()])?;
        parse_bundle_id(&info)
    }

    fn idle(&self) -> Option<SignedDuration> {
        let output = run("ioreg", &["-c", "IOHIDSystem", "-d", "4"])?;
        parse_hid_idle(&output)
    }

    fn process_running(&self, name: &str) -> bool {
        Command::new("pgrep")
            .args(["-x", name])
            .output()
            .is_ok_and(|output| output.status.success())
    }

    fn preview_front_document(&self) -> Option<PathBuf> {
        let path = run(
            "osascript",
            &[
                "-e",
                "tell application \"Preview\" to if (count of documents) > 0 then get path of front document",
            ],
        )?;
        let path = path.trim();
        (!path.is_empty()).then(|| PathBuf::from(path))
    }

    fn page_count(&self, path: &Path) -> Option<u32> {
        mdls(path, "kMDItemNumberOfPages")?.parse().ok()
    }

    fn document_title(&self, path: &Path) -> Option<String> {
        mdls(path, "kMDItemTitle").filter(|title| useful_title(title))
    }
}

fn mdls(path: &Path, attribute: &str) -> Option<String> {
    let output = Command::new("mdls")
        .args(["-raw", "-name", attribute])
        .arg(path)
        .output()
        .ok()?;
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !value.is_empty() && value != "(null)").then_some(value)
}

fn useful_title(title: &str) -> bool {
    let lower = title.to_lowercase();
    !lower.is_empty()
        && lower != "untitled"
        && !lower.starts_with("microsoft word")
        && !lower.ends_with(".pdf")
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn parse_bundle_id(info: &str) -> Option<String> {
    let value = info.split_once('=')?.1.trim().trim_matches('"');
    (!value.is_empty() && value != "[ NULL ]").then(|| value.to_string())
}

fn parse_hid_idle(ioreg: &str) -> Option<SignedDuration> {
    let line = ioreg
        .lines()
        .find(|line| line.contains("\"HIDIdleTime\""))?;
    let nanos: i64 = line.rsplit('=').next()?.trim().parse().ok()?;
    Some(SignedDuration::from_nanos(nanos))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_id_is_read_from_lsappinfo_output() {
        // given
        let info = "\"CFBundleIdentifier\"=\"com.apple.Preview\"\n";

        // when
        let bundle = parse_bundle_id(info);

        // then
        assert_eq!(bundle.as_deref(), Some("com.apple.Preview"));
    }

    #[test]
    fn placeholder_titles_are_not_useful() {
        // given
        let titles = [
            "Untitled",
            "Microsoft Word - draft.docx",
            "scan.pdf",
            "The Origin of Species",
        ];

        // when
        let useful: Vec<bool> = titles.iter().map(|t| useful_title(t)).collect();

        // then
        assert_eq!(useful, [false, false, false, true]);
    }

    #[test]
    fn idle_time_is_read_from_ioreg_nanoseconds() {
        // given
        let ioreg = "    | |     \"HIDIdleTime\" = 90000000000\n";

        // when
        let idle = parse_hid_idle(ioreg);

        // then
        assert_eq!(idle, Some(SignedDuration::from_nanos(90_000_000_000)));
    }
}
