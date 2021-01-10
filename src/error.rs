use cargo_toml::Error as CargoTomlError;
use rpm::RPMError;
use std::io::Error as IoError;
use std::path::PathBuf;

#[derive(thiserror::Error, Debug, Clone)]
pub enum ConfigError {
    #[error("Missing field: {0}")]
    Missing(&'static str),
    #[error("Field {0} must be {1}")]
    WrongType(&'static str, &'static str),
    #[error("{0} of {1}-th asset is undefined")]
    AssetFileUndefined(usize, &'static str),
    #[error("{1} of {0}-th asset must be {2}")]
    AssetFileWrongType(usize, &'static str, &'static str),
    #[error("Asset file not found: {0}")]
    AssetFileNotFound(String),
    #[error("Invalid glob at {0}: {1}")]
    AssetGlobInvalid(usize, &'static str),
    #[error("File unreadable: {0}")]
    AssetReadFailed(&'static str),
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    CargoToml(#[from] CargoTomlError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Rpm(#[from] RPMError),
    #[error("{1}: {0}")]
    FileIo(PathBuf, #[source] IoError),
    #[error(transparent)]
    Io(#[from] IoError),
}
