use anyhow::Result;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};
use readr_core::config::Config;
use readr_core::db::Db;
use readr_core::paths::Paths;

pub struct App {
    pub paths: Paths,
    pub config: Config,
    pub db: Db,
    pub tz: TimeZone,
}

impl App {
    pub fn load() -> Result<Self> {
        let paths = Paths::from_process_env()?;
        let db = Db::open(&paths.db_file)?;
        let config = match db.config()? {
            Some(config) => config,
            None => {
                let config = Config::load(&paths.config_file)?;
                db.save_config(&config)?;
                config
            }
        };
        Ok(Self {
            paths,
            config,
            db,
            tz: TimeZone::system(),
        })
    }

    pub fn now(&self) -> Zoned {
        Timestamp::now().to_zoned(self.tz.clone())
    }

    pub fn today(&self) -> Date {
        self.now().date()
    }

    pub fn local(&self, timestamp: Timestamp) -> Zoned {
        timestamp.to_zoned(self.tz.clone())
    }
}
