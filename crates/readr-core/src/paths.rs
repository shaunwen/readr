use std::path::PathBuf;

use anyhow::{Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub home: PathBuf,
    pub config_file: PathBuf,
    pub data_dir: PathBuf,
    pub db_file: PathBuf,
}

impl Paths {
    pub fn from_env(env: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let Some(home) = env("HOME").filter(|home| !home.is_empty()) else {
            bail!("HOME is not set");
        };
        let home = PathBuf::from(home);
        let config_dir = env("XDG_CONFIG_HOME")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("readr");
        let data_dir = env("XDG_DATA_HOME")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("readr");
        Ok(Self {
            config_file: config_dir.join("config.toml"),
            db_file: data_dir.join("readr.db"),
            data_dir,
            home,
        })
    }

    pub fn from_process_env() -> Result<Self> {
        Self::from_env(|key| std::env::var(key).ok())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn defaults_live_under_home() {
        // given
        let env = env_of(&[("HOME", "/Users/reader")]);

        // when
        let paths = Paths::from_env(env).unwrap();

        // then
        assert_eq!(
            paths.config_file,
            PathBuf::from("/Users/reader/.config/readr/config.toml")
        );
        assert_eq!(
            paths.db_file,
            PathBuf::from("/Users/reader/.local/share/readr/readr.db")
        );
    }

    #[test]
    fn xdg_dirs_override_defaults() {
        // given
        let env = env_of(&[
            ("HOME", "/Users/reader"),
            ("XDG_CONFIG_HOME", "/tmp/cfg"),
            ("XDG_DATA_HOME", "/tmp/data"),
        ]);

        // when
        let paths = Paths::from_env(env).unwrap();

        // then
        assert_eq!(
            paths.config_file,
            PathBuf::from("/tmp/cfg/readr/config.toml")
        );
        assert_eq!(paths.db_file, PathBuf::from("/tmp/data/readr/readr.db"));
    }

    #[test]
    fn home_is_required() {
        // given
        let env = env_of(&[]);

        // when
        let error = Paths::from_env(env).unwrap_err();

        // then
        assert!(error.to_string().contains("HOME"));
    }
}
