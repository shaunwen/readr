use std::path::Path;

use anyhow::{Result, bail};
use readr_core::config::Config;
use readr_core::settings::setting;

use crate::context::App;

pub fn show(app: &App) -> String {
    app.config.to_toml()
}

pub fn set(app: &App, key: &str, value: &str) -> Result<String> {
    let Some(field) = setting(key) else {
        bail!("unknown setting {key:?}; `readr config` lists them");
    };
    let config = app.config.with_setting(key, value)?;
    app.db.save_config(&config)?;
    Ok(format!(
        "{} = {}",
        field.label,
        shown(&config.get(key).unwrap_or_default())
    ))
}

pub fn import(app: &App, file: &Path) -> Result<String> {
    let config = Config::load_file(file)?;
    app.db.save_config(&config)?;
    let mut out = format!(
        "imported {} with {} goals\n",
        file.display(),
        config.goals.len()
    );
    let orphaned: Vec<String> = app
        .db
        .goals_in_use()?
        .into_iter()
        .filter(|key| config.goal(key).is_none())
        .collect();
    if !orphaned.is_empty() {
        out.push_str(&format!(
            "books or sessions still use goals that are gone: {}\n",
            orphaned.join(", ")
        ));
    }
    Ok(out)
}

fn shown(value: &str) -> &str {
    if value.is_empty() { "(not set)" } else { value }
}
