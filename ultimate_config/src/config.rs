use miette::Diagnostic;
use notify::{RecursiveMode, Watcher, recommended_watcher};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serenity::all::UserId;
use snafu::{ResultExt, Snafu};
use std::{
    collections::HashMap,
    fs::{read_to_string, write},
    path::{Path, PathBuf},
    sync::LazyLock,
    thread,
    time::{Duration, Instant},
};
use tracing::{error, info};

type Result<T, E = Error> = std::result::Result<T, E>;

const CONFIG_PATH: &str = "config.toml";

pub static CONFIG: LazyLock<RwLock<Config>> = LazyLock::new(|| {
    std::thread::spawn(watch_config);
    RwLock::new(Config::load().expect("valid config"))
});

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    model: String,
    name_substitutions: HashMap<UserId, String>,
}

#[derive(Debug, Snafu, Diagnostic)]
enum Error {
    #[snafu(display("Error while creating watcher: {source}"))]
    CreateWatcher { source: notify::Error },
    #[snafu(display("Error while watching config: {source}"))]
    BeginWatching { source: notify::Error },
    #[snafu(display("Error while reading config: {source}"))]
    Read { source: std::io::Error },
    #[snafu(display("Error while deserializing config: {source}"))]
    Deserialize { source: toml::de::Error },
    #[snafu(display("Error while saving config: {source}"))]
    Write { source: std::io::Error },
    #[snafu(display("Error while serializing config: {source}"))]
    Serialize { source: toml::ser::Error },
}

impl Config {
    fn load() -> Result<Self> {
        if Path::new(CONFIG_PATH).exists() {
            read_to_string(CONFIG_PATH)
                .context(ReadSnafu)
                .and_then(|contents| toml::from_str(&contents).context(DeserializeSnafu))
        } else {
            Ok(Self::default())
        }
    }

    fn save(&self) -> Result<()> {
        toml::to_string_pretty(self)
            .context(SerializeSnafu)
            .and_then(|contents| write(CONFIG_PATH, contents).context(WriteSnafu))
    }

    pub fn model(&self) -> String {
        self.model.clone()
    }

    pub fn substitute_name(&self, user_id: UserId) -> String {
        self.name_substitutions
            .get(&user_id)
            .cloned()
            .unwrap_or_else(|| "User".to_owned())
    }
}

impl Default for Config {
    fn default() -> Self {
        let config = Self {
            model: "deepseek-chat".to_owned(),
            name_substitutions: HashMap::new(),
        };
        if !Path::new(CONFIG_PATH).exists() {
            config.save().ok();
        }
        config
    }
}

// TODO: remove
#[allow(clippy::cognitive_complexity)]
fn watch_config() -> Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = recommended_watcher(tx).context(CreateWatcherSnafu)?;
    let config_path = PathBuf::from(CONFIG_PATH);
    let mut last_reload = Instant::now();
    let debounce_duration = Duration::from_millis(400);
    let post_event_delay = Duration::from_millis(1);

    let watch_path = config_path.as_ref();
    watcher
        .watch(watch_path, RecursiveMode::NonRecursive)
        .context(BeginWatchingSnafu)?;

    for res in rx {
        match res {
            Err(e) => error!("Watch error: {e:?}"),
            Ok(event) => {
                let config_file_exists = config_path.exists();
                let event_is_remove = event.kind.is_remove();
                let debounced = Instant::now().duration_since(last_reload) >= debounce_duration;

                if event_is_remove && config_file_exists {
                    if let Err(e) = watcher.watch(watch_path, RecursiveMode::NonRecursive) {
                        error!("Failed to re-establish watch: {e:?}");
                    }
                }

                if debounced && config_file_exists {
                    thread::sleep(post_event_delay);
                    match Config::load() {
                        Err(e) => error!("Failed to reload config: {e:?}"),
                        Ok(new_config) => {
                            *CONFIG.write() = new_config;
                            info!("Config reloaded");
                            last_reload = Instant::now();
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
