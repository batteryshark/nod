use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct DesktopPreferences {
    pub hide_notification_content: bool,
    pub load_remote_images: bool,
    pub muted_channels: Vec<String>,
    pub snoozed_until: Option<i64>,
    pub quiet_start_hour: Option<u8>,
    pub quiet_end_hour: Option<u8>,
}

impl DesktopPreferences {
    pub(crate) fn notifications_paused(&self, channel_key: &str, now: i64, hour: u8) -> bool {
        if self.snoozed_until.is_some_and(|until| until > now)
            || self
                .muted_channels
                .iter()
                .any(|channel| channel == channel_key)
        {
            return true;
        }
        match (self.quiet_start_hour, self.quiet_end_hour) {
            (Some(start), Some(end)) if start < end => (start..end).contains(&hour),
            (Some(start), Some(end)) if start > end => hour >= start || hour < end,
            _ => false,
        }
    }

    fn validate(&self) -> Result<()> {
        if self.quiet_start_hour.is_some_and(|hour| hour > 23)
            || self.quiet_end_hour.is_some_and(|hour| hour > 23)
            || self.quiet_start_hour.is_some() != self.quiet_end_hour.is_some()
        {
            bail!("Choose both quiet-hours endpoints between 0 and 23");
        }
        if self.muted_channels.len() > 10_000 {
            bail!("Too many muted channels");
        }
        Ok(())
    }
}

pub(crate) struct PreferenceStore {
    path: PathBuf,
    value: DesktopPreferences,
}

impl PreferenceStore {
    pub(crate) fn open(directory: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&directory)?;
        let path = directory.join("desktop-preferences.json");
        let value = match std::fs::read(&path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).context("Cannot read desktop preferences")?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                DesktopPreferences::default()
            }
            Err(error) => return Err(error.into()),
        };
        Ok(Self { path, value })
    }

    pub(crate) fn get(&self) -> DesktopPreferences {
        self.value.clone()
    }

    pub(crate) fn save(&mut self, value: DesktopPreferences) -> Result<DesktopPreferences> {
        value.validate()?;
        let temporary = self.path.with_extension("json.tmp");
        let mut file = std::fs::File::create(&temporary)?;
        serde_json::to_writer_pretty(&mut file, &value)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(temporary, &self.path)?;
        self.value = value;
        Ok(self.get())
    }
}

#[cfg(test)]
mod tests {
    use super::DesktopPreferences;

    #[test]
    fn overnight_quiet_hours_end_at_the_configured_hour() {
        let preferences = DesktopPreferences {
            quiet_start_hour: Some(22),
            quiet_end_hour: Some(7),
            ..Default::default()
        };
        assert!(preferences.notifications_paused("server:channel", 0, 23));
        assert!(preferences.notifications_paused("server:channel", 0, 6));
        assert!(!preferences.notifications_paused("server:channel", 0, 7));
    }

    #[test]
    fn channel_mute_is_scoped_to_a_server() {
        let preferences = DesktopPreferences {
            muted_channels: vec!["one:deploy".into()],
            ..Default::default()
        };
        assert!(!preferences.notifications_paused("two:deploy", 0, 12));
    }
}
