use std::{
    env,
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};

pub(super) fn default_home_config_path(home_relative_path: &str) -> Option<PathBuf> {
    env::var_os("HOME").map(|home| PathBuf::from(home).join(home_relative_path))
}

pub(crate) fn default_config_locations(
    home_path: Option<&Path>,
    cwd_path: Option<&Path>,
) -> Vec<String> {
    let mut locations = Vec::new();
    if let Some(path) = home_path {
        locations.push(path.display().to_string());
    }
    if let Some(path) = cwd_path {
        locations.push(path.display().to_string());
    }
    locations
}

pub(super) fn default_config_update_path(
    home_path: Option<&Path>,
    cwd_path: Option<&Path>,
) -> Result<PathBuf> {
    if let Some(path) = cwd_path.filter(|path| path.exists()) {
        return Ok(path.to_path_buf());
    }

    if let Some(path) = home_path {
        return Ok(path.to_path_buf());
    }

    if let Some(path) = cwd_path {
        return Ok(path.to_path_buf());
    }

    bail!("failed to determine a config path for config updates")
}
