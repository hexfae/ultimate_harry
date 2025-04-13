use miette::Diagnostic;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serenity::all::UserId;
use snafu::{ResultExt, Snafu};
use std::{
    collections::HashMap,
    fs::{read_to_string, write},
    path::Path,
    sync::LazyLock,
};
use tracing::warn;

type Result<T, E = Error> = std::result::Result<T, E>;

const STATISTICS_PATH: &str = "statistics.toml";

pub static STATISTICS: LazyLock<RwLock<Statistics>> =
    LazyLock::new(|| RwLock::new(Statistics::load().expect("valid statistics")));

#[derive(Serialize, Deserialize)]
pub struct Statistics(HashMap<UserId, UserStatistic>);

#[derive(Default, Serialize, Deserialize)]
struct UserStatistic {
    characters_created: u32,
    characters_edited: u32,
    characters_viewed: u32,
    characters_deleted: u32,
    conversations_started: u32,
}

#[derive(Debug, Snafu, Diagnostic)]
enum Error {
    #[snafu(display("Error while reading config: {source}"))]
    Read { source: std::io::Error },
    #[snafu(display("Error while deserializing config: {source}"))]
    Deserialize { source: toml::de::Error },
    #[snafu(display("Error while saving config: {source}"))]
    Write { source: std::io::Error },
    #[snafu(display("Error while serializing config: {source}"))]
    Serialize { source: toml::ser::Error },
}

impl Statistics {
    fn load() -> Result<Self> {
        if Path::new(STATISTICS_PATH).exists() {
            read_to_string(STATISTICS_PATH)
                .context(ReadSnafu)
                .and_then(|contents| toml::from_str(&contents).context(DeserializeSnafu))
        } else {
            Ok(Self::default())
        }
    }

    fn save(&self) {
        if let Err(why) = toml::to_string_pretty(self)
            .context(SerializeSnafu)
            .and_then(|contents| write(STATISTICS_PATH, contents).context(WriteSnafu))
        {
            warn!("Error while saving statistics: {why}");
        }
    }

    pub fn character_created_by(&mut self, user: impl Into<UserId>) {
        self.0
            .entry(user.into())
            .and_modify(|statistic| {
                statistic.characters_created += 1;
            })
            .or_insert_with(|| UserStatistic {
                characters_created: 1,
                ..Default::default()
            });
        self.save();
    }

    pub fn character_edited_by(&mut self, user: impl Into<UserId>) {
        self.0
            .entry(user.into())
            .and_modify(|statistic| {
                statistic.characters_edited += 1;
            })
            .or_insert_with(|| UserStatistic {
                characters_edited: 1,
                ..Default::default()
            });
        self.save();
    }

    pub fn character_viewed_by(&mut self, user: impl Into<UserId>) {
        self.0
            .entry(user.into())
            .and_modify(|statistic| {
                statistic.characters_viewed += 1;
            })
            .or_insert_with(|| UserStatistic {
                characters_viewed: 1,
                ..Default::default()
            });
        self.save();
    }

    pub fn character_deleted_by(&mut self, user: impl Into<UserId>) {
        self.0
            .entry(user.into())
            .and_modify(|statistic| {
                statistic.characters_deleted += 1;
            })
            .or_insert_with(|| UserStatistic {
                characters_deleted: 1,
                ..Default::default()
            });
        self.save();
    }

    pub fn conversation_started_by(&mut self, user: impl Into<UserId>) {
        self.0
            .entry(user.into())
            .and_modify(|statistic| {
                statistic.conversations_started += 1;
            })
            .or_insert_with(|| UserStatistic {
                conversations_started: 1,
                ..Default::default()
            });
        self.save();
    }
}

impl Default for Statistics {
    fn default() -> Self {
        let statistics = Self(HashMap::new());
        if !Path::new(STATISTICS_PATH).exists() {
            statistics.save();
        }
        statistics
    }
}
