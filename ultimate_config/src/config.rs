use miette::Diagnostic;
use notify::{RecursiveMode, Watcher, recommended_watcher};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serenity::all::{ChannelId, GuildId, UserId};
use snafu::{ResultExt, Snafu};
use std::{
    collections::HashMap,
    env::var,
    fs::{read_to_string, write},
    path::{Path, PathBuf},
    sync::LazyLock,
    thread::sleep,
    time::{Duration, Instant},
};
use tracing::{error, info, warn};

type Result<T, E = Error> = std::result::Result<T, E>;

pub static CONFIG_PATH: LazyLock<String> =
    LazyLock::new(|| var("CONFIG_FILE").unwrap_or_else(|_| "config.toml".to_owned()));

pub static CONFIG: LazyLock<RwLock<Config>> = LazyLock::new(|| {
    std::thread::spawn(watch_config);
    RwLock::new(Config::load().expect("valid config"))
});

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    model_settings: ModelSettings,
    name_substitutions: HashMap<UserId, String>,
    bot_token: String,
    guild_ids: Vec<GuildId>,
    pins_channel_id: ChannelId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSettings {
    model: String,
    api_key: String,
    api_base: String,
    /// The default frequency penalty for requests.
    frequency_penalty: f32,
    /// The default presence penalty for requests
    presence_penalty: f32,
    /// The default temperature for requests.
    temperature: f32,
    /// The default top-p value for requests.
    top_p: f32,
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
        if Path::new(&CONFIG_PATH.clone()).exists() {
            read_to_string(CONFIG_PATH.clone())
                .context(ReadSnafu)
                .and_then(|contents| toml::from_str(&contents).context(DeserializeSnafu))
        } else {
            Ok(Self::default())
        }
    }

    fn save(&self) {
        if let Err(why) = toml::to_string_pretty(self)
            .context(SerializeSnafu)
            .and_then(|contents| write(CONFIG_PATH.clone(), contents).context(WriteSnafu))
        {
            warn!("Error while saving config: {why}");
        }
    }

    pub fn bot_token(&self) -> String {
        self.bot_token.clone()
    }

    pub fn guild_ids(&self) -> Vec<GuildId> {
        self.guild_ids.clone()
    }

    pub const fn pins_channel_id(&self) -> ChannelId {
        self.pins_channel_id
    }

    pub fn model_settings(&self) -> ModelSettings {
        self.model_settings.clone()
    }

    pub fn substitute_name(&self, user_id: impl AsRef<UserId>) -> String {
        self.name_substitutions
            .get(user_id.as_ref())
            .cloned()
            .unwrap_or_else(|| "User".to_owned())
    }
}

impl ModelSettings {
    #[must_use]
    pub fn model(&self) -> String {
        self.model.clone()
    }

    #[must_use]
    pub fn api_key(&self) -> String {
        self.api_key.clone()
    }

    #[must_use]
    pub fn api_base(&self) -> String {
        self.api_base.clone()
    }

    #[must_use]
    pub const fn frequency_penalty(&self) -> f32 {
        self.frequency_penalty
    }

    #[must_use]
    pub const fn presence_penalty(&self) -> f32 {
        self.presence_penalty
    }

    #[must_use]
    pub const fn temperature(&self) -> f32 {
        self.temperature
    }

    #[must_use]
    pub const fn top_p(&self) -> f32 {
        self.top_p
    }
}

impl Default for Config {
    fn default() -> Self {
        let config = Self {
            pins_channel_id: ChannelId::new(1),
            guild_ids: vec![GuildId::new(1)],
            bot_token: String::new(),
            name_substitutions: HashMap::new(),
            model_settings: ModelSettings {
                model: String::new(),
                api_key: String::new(),
                api_base: String::new(),
                // TODO: what are default values for all of these?
                frequency_penalty: 0.0,
                presence_penalty: 0.0,
                temperature: 0.0,
                top_p: 0.0,
            },
        };
        if !Path::new(&CONFIG_PATH.clone()).exists() {
            config.save();
        }
        config
    }
}

// TODO: remove
#[allow(clippy::cognitive_complexity)]
fn watch_config() -> Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = recommended_watcher(tx).context(CreateWatcherSnafu)?;
    let config_path = PathBuf::from(CONFIG_PATH.clone());
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
                    sleep(post_event_delay);
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
