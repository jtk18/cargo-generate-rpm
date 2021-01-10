use crate::error::{ConfigError, Error};
use cargo_toml::Error as CargoTomlError;
use cargo_toml::Manifest;
use rpm::{Compressor, RPMBuilder, RPMFileOptions};
use std::env::consts::ARCH;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use toml::value::Table;

pub type ConfigTable = toml::map::Map<std::string::String, cargo_toml::Value>;

#[derive(Debug)]
pub struct Config {
    manifest: Manifest,
    path: PathBuf,
}

impl Config {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref().to_path_buf();
        Manifest::from_path(&path)
            .map(|manifest| Config {
                manifest,
                path: path.clone(),
            })
            .map_err(|err| match err {
                CargoTomlError::Io(e) => Error::FileIo(path, e),
                _ => Error::CargoToml(err),
            })
    }

    fn metadata(&self) -> Result<&Table, ConfigError> {
        let pkg = self
            .manifest
            .package
            .as_ref()
            .ok_or(ConfigError::Missing("package"))?;
        let metadata = pkg
            .metadata
            .as_ref()
            .ok_or(ConfigError::Missing("package.metadata"))?
            .as_table()
            .ok_or(ConfigError::WrongType("package.metadata", "table"))?;
        let metadata = metadata
            .iter()
            .find(|(name, _)| name.as_str() == "generate-rpm")
            .ok_or(ConfigError::Missing("package.metadata.generate-rpm"))?
            .1
            .as_table()
            .ok_or(ConfigError::WrongType(
                "package.metadata.generate-rpm",
                "table",
            ))?;
        Ok(metadata)
    }

    fn files(&self) -> Result<Vec<FileInfo>, ConfigError> {
        let metadata = self.metadata()?;
        let assets = metadata
            .get("assets")
            .ok_or(ConfigError::Missing("package.metadata.generate-rpm.assets"))?
            .as_array()
            .ok_or(ConfigError::WrongType(
                "package.metadata.generate-rpm.assets",
                "array",
            ))?;

        let mut files = Vec::with_capacity(assets.len());
        for (idx, value) in assets.iter().enumerate() {
            let table = value
                .as_table()
                .ok_or(ConfigError::AssetFileUndefined(idx, "source"))?
                .clone();
            let info = _handle_file(table, idx)?;

            files.push(info);
        }
        Ok(files)
    }

    pub fn create_rpm_builder(&self, target_arch: Option<String>) -> Result<RPMBuilder, Error> {
        let metadata = self.metadata()?;
        macro_rules! get_str_from_metadata {
            ($name:expr) => {
                if let Some(val) = metadata.get($name) {
                    Some(val.as_str()
                        .ok_or(ConfigError::WrongType(
                            concat!("package.metadata.generate-rpm.", $name),
                            "string"
                        ))?)
                } else {
                    None
                } as Option<&str>
            }
        };
        macro_rules! get_i64_from_metadata {
            ($name:expr) => {
                if let Some(val) = metadata.get($name) {
                    Some(val.as_integer()
                        .ok_or(ConfigError::WrongType(
                            concat!("package.metadata.generate-rpm.", $name),
                            "integer"
                        ))?)
                } else {
                    None
                } as Option<i64>
            }
        };

        let pkg = self
            .manifest
            .package
            .as_ref()
            .ok_or(ConfigError::Missing("package"))?;
        let name = get_str_from_metadata!("name").unwrap_or(pkg.name.as_str());
        let version = get_str_from_metadata!("version").unwrap_or(pkg.version.as_str());
        let license = get_str_from_metadata!("license").unwrap_or(
            pkg.license
                .as_ref()
                .ok_or(ConfigError::Missing("package.license"))?
                .as_str(),
        );
        let arch = target_arch.unwrap_or(
            match ARCH {
                "x86" => "i586",
                "arm" => "armhfp",
                "powerpc" => "ppc",
                "powerpc64" => "ppc64",
                _ => ARCH,
            }
            .to_string(),
        );
        let desc = get_str_from_metadata!("summary").unwrap_or(
            pkg.description
                .as_ref()
                .ok_or(ConfigError::Missing("package.description"))?
                .as_str(),
        );

        let mut builder = RPMBuilder::new(name, version, license, arch.as_str(), desc)
            .compression(Compressor::from_str("gzip").unwrap());
        for file in &self.files()? {
            let options = file.generate_rpm_file_options();

            let file_source = [
                PathBuf::from(&file.source),
                self.path.parent().unwrap().join(&file.source),
            ]
            .iter()
            .find(|v| v.exists())
            .ok_or(ConfigError::AssetFileNotFound(file.source.to_string()))?
            .to_owned();

            builder = builder.with_file(file_source, options)?;
        }

        if let Some(release) = get_i64_from_metadata!("release") {
            builder = builder.release(release as u16);
        }
        if let Some(epoch) = get_i64_from_metadata!("epoch") {
            builder = builder.epoch(epoch as i32);
        }

        if let Some(pre_install_script) = get_str_from_metadata!("pre_install_script") {
            builder = builder.pre_install_script(pre_install_script);
        }
        if let Some(pre_uninstall_script) = get_str_from_metadata!("pre_uninstall_script") {
            builder = builder.pre_uninstall_script(pre_uninstall_script);
        }
        if let Some(post_install_script) = get_str_from_metadata!("post_install_script") {
            builder = builder.post_install_script(post_install_script);
        }
        if let Some(post_uninstall_script) = get_str_from_metadata!("post_uninstall_script") {
            builder = builder.post_uninstall_script(post_uninstall_script);
        }

        Ok(builder)
    }
}

fn _handle_file(table: ConfigTable, idx: usize) -> Result<FileInfo, ConfigError> {
    let source = _get_source(&table, idx)?;
    let dest = _get_dest(&table, idx)?;

    let user = _get_user(&table, idx)?;
    let group = _get_group(&table, idx)?;
    let mode = _get_mode(&table, &source, idx)?;
    let config = _get_config(&table, idx)?;
    let doc = _get_doc(&table, idx)?;

    let info = FileInfo {
        source,
        dest,
        user,
        group,
        mode,
        config,
        doc,
    };
    Ok(info)
}

fn _get_source(table: &ConfigTable, idx: usize) -> Result<String, ConfigError> {
    Ok(table
        .get("source")
        .ok_or(ConfigError::AssetFileUndefined(idx, "source"))?
        .as_str()
        .ok_or(ConfigError::AssetFileWrongType(idx, "source", "string"))?
        .to_owned())
}

fn _get_dest(table: &ConfigTable, idx: usize) -> Result<String, ConfigError> {
    Ok(table
        .get("dest")
        .ok_or(ConfigError::AssetFileUndefined(idx, "dest"))?
        .as_str()
        .ok_or(ConfigError::AssetFileWrongType(idx, "dest", "string"))?
        .to_owned())
}

fn _get_user(table: &ConfigTable, idx: usize) -> Result<Option<String>, ConfigError> {
    if let Some(user) = table.get("user") {
        Ok(Some(
            user.as_str()
                .ok_or(ConfigError::AssetFileWrongType(idx, "user", "string"))?
                .to_owned(),
        ))
    } else {
        Ok(None)
    }
}

fn _get_group(table: &ConfigTable, idx: usize) -> Result<Option<String>, ConfigError> {
    if let Some(group) = table.get("group") {
        Ok(Some(
            group
                .as_str()
                .ok_or(ConfigError::AssetFileWrongType(idx, "group", "string"))?
                .to_owned(),
        ))
    } else {
        Ok(None)
    }
}

fn _get_mode(table: &ConfigTable, source: &str, idx: usize) -> Result<Option<usize>, ConfigError> {
    if let Some(mode) = table.get("mode") {
        let mode = mode
            .as_str()
            .ok_or(ConfigError::AssetFileWrongType(idx, "mode", "string"))?;
        let mode = usize::from_str_radix(mode, 8)
            .map_err(|_| ConfigError::AssetFileWrongType(idx, "mode", "oct-string"))?;
        let file_mode = if mode & 0o170000 != 0 {
            None
        } else if source.ends_with('/') {
            Some(0o040000) // S_IFDIR
        } else {
            Some(0o100000) // S_IFREG
        };
        Ok(Some(file_mode.unwrap_or_default() | mode))
    } else {
        Ok(None)
    }
}

fn _get_config(table: &ConfigTable, idx: usize) -> Result<bool, ConfigError> {
    if let Some(is_config) = table.get("config") {
        is_config
            .as_bool()
            .ok_or(ConfigError::AssetFileWrongType(idx, "config", "bool"))
    } else {
        Ok(false)
    }
}

fn _get_doc(table: &ConfigTable, idx: usize) -> Result<bool, ConfigError> {
    if let Some(is_doc) = table.get("doc") {
        is_doc
            .as_bool()
            .ok_or(ConfigError::AssetFileWrongType(idx, "doc", "bool"))
    } else {
        Ok(false)
    }
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct FileInfo {
    source: String,
    dest: String,
    user: Option<String>,
    group: Option<String>,
    mode: Option<usize>,
    config: bool,
    doc: bool,
}

impl FileInfo {
    fn generate_rpm_file_options(&self) -> RPMFileOptions {
        let mut rpm_file_option = RPMFileOptions::new(&self.dest);
        if let Some(user) = &self.user {
            rpm_file_option = rpm_file_option.user(user);
        }
        if let Some(group) = &self.group {
            rpm_file_option = rpm_file_option.group(group);
        }
        if let Some(mode) = self.mode {
            rpm_file_option = rpm_file_option.mode(mode as i32);
        }
        if self.config {
            rpm_file_option = rpm_file_option.is_config();
        }
        if self.doc {
            rpm_file_option = rpm_file_option.is_doc();
        }
        rpm_file_option.into()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_config_new() {
        let config = Config::new("Cargo.toml").unwrap();
        let pkg = config.manifest.package.unwrap();
        assert_eq!(pkg.name, "cargo-generate-rpm");

        assert!(matches!(Config::new("not_exist_path/Cargo.toml"),
            Err(Error::FileIo(path, error)) if path == PathBuf::from("not_exist_path/Cargo.toml") && error.kind() == std::io::ErrorKind::NotFound));
        assert!(matches!(
            Config::new("src/error.rs"),
            Err(Error::CargoToml(_))
        ));
    }

    #[test]
    fn test_metadata() {
        let config = Config::new("Cargo.toml").unwrap();
        let metadata = config.metadata().unwrap();
        let assets = metadata.get("assets").unwrap();
        assert!(assets.is_array());
    }

    #[test]
    fn test_files() {
        let config = Config::new("Cargo.toml").unwrap();
        let files = config.files().unwrap();
        assert_eq!(
            files,
            vec![
                FileInfo {
                    source: "target/release/cargo-generate-rpm".to_owned(),
                    dest: "/usr/bin/cargo-generate-rpm".to_owned(),
                    user: None,
                    group: None,
                    mode: Some(0o0100755),
                    config: false,
                    doc: false
                },
                FileInfo {
                    source: "LICENSE".to_owned(),
                    dest: "/usr/share/doc/cargo-generate-rpm/LICENSE".to_owned(),
                    user: None,
                    group: None,
                    mode: Some(0o0100644),
                    config: false,
                    doc: true
                },
                FileInfo {
                    source: "README.md".to_owned(),
                    dest: "/usr/share/doc/cargo-generate-rpm/README.md".to_owned(),
                    user: None,
                    group: None,
                    mode: Some(0o0100644),
                    config: false,
                    doc: true
                }
            ]
        );
    }

    #[test]
    fn test_config_create_rpm_builder() {
        let config = Config::new("Cargo.toml").unwrap();
        let builder = config.create_rpm_builder(None);

        assert!(if Path::new("target/release/cargo-generate-rpm").exists() {
            matches!(builder, Ok(_))
        } else {
            matches!(builder, Err(Error::Config(ConfigError::AssetFileNotFound(path))) if path == "target/release/cargo-generate-rpm")
        });
    }
}
