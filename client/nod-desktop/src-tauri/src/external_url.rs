use anyhow::{bail, Context, Result};
use url::Url;

pub(crate) fn open_url(value: &str) -> Result<()> {
    launch_url(&web_url(value)?)
}

pub(crate) fn web_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).context("The link must be an absolute HTTP or HTTPS URL")?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("Only HTTP and HTTPS links can be opened");
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("Links containing credentials cannot be opened");
    }
    Ok(url)
}

#[cfg(target_os = "windows")]
fn launch_url(url: &Url) -> Result<()> {
    use windows::{
        core::{w, PCWSTR},
        Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    };
    let url: Vec<u16> = url.as_str().encode_utf16().chain(Some(0)).collect();
    // The URL goes directly to its registered handler, never through cmd.exe.
    // The terminated UTF-16 buffer remains alive for the entire native call.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(url.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        bail!(
            "Windows could not open the link (error {})",
            result.0 as isize
        );
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn launch_url(url: &Url) -> Result<()> {
    #[cfg(target_os = "linux")]
    let opener = "xdg-open";
    #[cfg(target_os = "macos")]
    let opener = "/usr/bin/open";
    std::process::Command::new(opener)
        .arg(url.as_str())
        .spawn()
        .context("Could not launch the default browser")?;
    Ok(())
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn launch_url(_url: &Url) -> Result<()> {
    bail!("Opening links is unavailable on this platform")
}

#[cfg(test)]
mod tests {
    use super::web_url;

    #[test]
    fn accepts_private_http_and_preserves_query_parameters() {
        let value = "http://127.0.0.1:8767/path?a=one&b=two%20words";
        assert_eq!(web_url(value).unwrap().as_str(), value);
    }

    #[test]
    fn rejects_non_web_schemes_and_relative_paths() {
        for value in [
            "javascript:alert(1)",
            "file:///tmp/request",
            "cmd.exe",
            "//example.com",
            "mailto:a@example.com",
        ] {
            assert!(web_url(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn rejects_embedded_credentials() {
        assert!(web_url("https://trusted.example@other.example/").is_err());
    }
}
