#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(target_os = "linux", test))]
mod notification_registry;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(super) use linux::{remove_notification, show_notification};
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub(super) use unsupported::{remove_notification, show_notification};
#[cfg(target_os = "windows")]
pub(crate) use windows::register_toast_app_id;
#[cfg(target_os = "windows")]
pub(super) use windows::{remove_notification, show_notification};
