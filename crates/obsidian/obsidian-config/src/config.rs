use std::path::Path;

use serde::Deserialize;

// our aim is to load values from the config toml file
// and then set the values to the config struct and initialise
// the engine using these config values

#[derive(Deserialize, Debug)]
pub struct ObsidianConfig {
    #[serde(default = "defaults::shm_file_path")]
    pub shm_file_path: String,
    #[serde(default = "defaults::log_level")]
    pub log_level: String,
    #[serde(default = "defaults::capacity")]
    pub capacity: usize,
    pub connections: Vec<ConnectionConfig>,
}

#[derive(Deserialize, Debug)]
pub struct ConnectionConfig {
    pub url: String,
    pub symbol_id: u16,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read '{path}'")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse config")]
    Parse(#[from] toml::de::Error),
}

mod defaults {
    pub fn shm_file_path() -> String {
        "/tmp/lithos_md_bus".into()
    }

    pub fn log_level() -> String {
        "info".into()
    }

    pub fn capacity() -> usize {
        return 1 << 16; // 65536
    }
}

impl ObsidianConfig {
    // this function takes path to the config as the input
    // result type should be a config error enum
    pub fn load(path: impl AsRef<Path> + ToString) -> Result<Self, ConfigError> {
        // first we load the contents on the toml file and then we read it ?
        // so in the toml crate we read the toml document as a str so we first need to convert
        // the toml file contents into string
        let toml_to_str = std::fs::read_to_string(&path).map_err(|source| ConfigError::Read {
            path: path.to_string(),
            source,
        })?;
        let onyx_config: ObsidianConfig = toml::from_str(&toml_to_str)?;
        Ok(onyx_config)
    }
}
