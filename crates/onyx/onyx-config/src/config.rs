use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize, Debug)]
pub struct OnyxConfig {
    #[serde(default = "defaults::shm_file_path")]
    pub shm_file_path: String,
    #[serde(default = "defaults::log_level")]
    pub log_level: String,
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
}

impl OnyxConfig {
    pub fn load(path: impl AsRef<Path> + ToString) -> Result<Self, ConfigError> {
        let toml_to_str = std::fs::read_to_string(&path).map_err(|source| ConfigError::Read {
            path: path.to_string(),
            source,
        })?;
        let onyx_config: OnyxConfig = toml::from_str(&toml_to_str)?;
        Ok(onyx_config)
    }
}
