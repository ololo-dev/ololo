//! `ololo profile list` / `ololo profile remove` commands.

use crate::config::{Config, list_profiles};
use anyhow::{Result, anyhow};

use crate::ui;

pub fn run_profile_list(active_profile: &str) -> Result<()> {
    let profiles = list_profiles().map_err(|e| anyhow!("{e}"))?;
    if profiles.is_empty() {
        ui::hint("No profiles configured. Run 'ololo login' to add one.");
        return Ok(());
    }
    let name_width = profiles.iter().map(|p| p.name.len()).max().unwrap_or(8);
    for p in &profiles {
        ui::profile_row(p.name == active_profile, &p.name, &p.server_url, name_width);
    }
    Ok(())
}

pub async fn run_profile_remove(name: &str) -> Result<()> {
    Config::delete(name).map_err(|e| anyhow!("{e}"))?;
    ui::success(format!("Profile '{name}' removed."));
    Ok(())
}

#[cfg(test)]
mod tests {

    #[test]
    fn profile_list_empty_is_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::config::env_lock().lock().unwrap();
        let _h = crate::test_util::test_util::HomeGuard::set(tmp.path().to_str().unwrap());
        let result = super::run_profile_list("default");
        assert!(result.is_ok());
    }
}
