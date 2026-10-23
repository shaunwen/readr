use std::path::Path;

use anyhow::{Result, bail};
use readr_core::config::Config;
use readr_core::model::BookStatus;
use readr_core::settings::{GOAL_FIELDS, setting};

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

pub fn set_goal_field(app: &App, goal: &str, field: &str, value: &str) -> Result<String> {
    let label = GOAL_FIELDS
        .iter()
        .find(|f| f.key == field)
        .map_or(field, |f| f.label);
    let config = app.config.with_goal_field(goal, field, value)?;
    if field == "key" {
        let new = value.trim();
        if new != goal {
            app.db.rename_goal(goal, new)?;
        }
    }
    app.db.save_config(&config)?;
    Ok(format!("{label} = {}", shown(value.trim())))
}

pub fn add_goal(app: &App, label: &str) -> Result<String> {
    let (config, key) = app.config.with_new_goal(label)?;
    app.db.save_config(&config)?;
    Ok(format!("added goal {} ({key})", label.trim()))
}

pub fn remove_goal(app: &App, key: &str) -> Result<String> {
    let label = app
        .config
        .goal(key)
        .map(|g| g.label.clone())
        .unwrap_or_else(|| key.into());
    let books = app
        .db
        .books()?
        .into_iter()
        .filter(|b| b.goal.as_deref() == Some(key))
        .filter(|b| matches!(b.status, BookStatus::Active | BookStatus::Queued))
        .count();
    match books {
        0 => {}
        1 => bail!("{label} still has a book on the shelf; give it another goal first"),
        n => bail!("{label} still has {n} books on the shelf; give them another goal first"),
    }
    app.db.save_config(&app.config.without_goal(key))?;
    Ok(format!(
        "removed goal {label}; past sessions keep their time"
    ))
}

pub fn move_goal(app: &App, key: &str, delta: isize) -> Result<String> {
    app.db
        .save_config(&app.config.with_goal_moved(key, delta))?;
    Ok(String::new())
}

fn shown(value: &str) -> &str {
    if value.is_empty() { "(not set)" } else { value }
}
