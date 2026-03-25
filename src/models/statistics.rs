use dashmap::DashMap;
use miette::Diagnostic;
use serde::{Deserialize, Serialize};
use serenity::all::UserId;
use snafu::{ResultExt as _, Snafu};
use std::{
    fs::{read_to_string, write},
    path::Path,
};
use tracing::warn;

type Result<T, E = StatisticsError> = std::result::Result<T, E>;

const STATISTICS_PATH: &str = "statistics.toml";

#[derive(Serialize, Deserialize)]
pub struct Statistics(DashMap<UserId, UserStatistic>);

#[derive(Default, Serialize, Deserialize)]
struct UserStatistic {
    characters_created: u32,
    characters_edited: u32,
    characters_viewed: u32,
    characters_deleted: u32,
    conversations_started: u32,
}

#[derive(Debug, Snafu, Diagnostic)]
pub enum StatisticsError {
    #[snafu(display("Error while reading statistics: {source}"))]
    Read { source: std::io::Error },
    #[snafu(display("Error while deserializing statistics: {source}"))]
    Deserialize { source: toml::de::Error },
    #[snafu(display("Error while saving statistics: {source}"))]
    Write { source: std::io::Error },
    #[snafu(display("Error while serializing statistics: {source}"))]
    Serialize { source: toml::ser::Error },
}

impl Statistics {
    pub fn load() -> Result<Self> {
        if Path::new(STATISTICS_PATH).exists() {
            let contents = read_to_string(STATISTICS_PATH).context(ReadSnafu)?;
            toml::from_str(&contents).context(DeserializeSnafu)
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

    pub fn character_created_by(&self, user: impl Into<UserId>) {
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

    pub fn character_edited_by(&self, user: impl Into<UserId>) {
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

    pub fn character_viewed_by(&self, user: impl Into<UserId>) {
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

    pub fn character_deleted_by(&self, user: impl Into<UserId>) {
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

    pub fn conversation_started_by(&self, user: impl Into<UserId>) {
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
        let statistics = Self(DashMap::new());
        if !Path::new(STATISTICS_PATH).exists() {
            statistics.save();
        }
        statistics
    }
}
