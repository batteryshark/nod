use crate::{desktop_state::DesktopState, external_url::web_url};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{io::Cursor, time::Duration};
use tauri::State;

const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
static IMAGE_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

#[tauri::command]
pub(crate) async fn request_image(
    state: State<'_, DesktopState>,
    url: String,
) -> Result<String, String> {
    if !state.preferences.lock().await.get().load_remote_images {
        return Err("Remote images are disabled in Settings".into());
    }
    fetch_image(&url).await.map_err(|error| error.to_string())
}

async fn fetch_image(value: &str) -> Result<String> {
    let _slot = IMAGE_SLOTS
        .try_acquire()
        .context("Image previews are busy; try again shortly")?;
    let url = web_url(value)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut response = client.get(url).send().await?.error_for_status()?;
    if response.status().is_redirection() {
        bail!("Image redirects are not loaded; open the attachment in a browser");
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_IMAGE_BYTES as u64)
    {
        bail!("Image exceeds the 8 MB limit");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len().saturating_add(chunk.len()) > MAX_IMAGE_BYTES {
            bail!("Image exceeds the 8 MB limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    tokio::task::spawn_blocking(move || image_preview(bytes))
        .await
        .context("Image preview task failed")?
}

fn image_preview(bytes: Vec<u8>) -> Result<String> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    if !matches!(
        reader.format(),
        Some(image::ImageFormat::Png | image::ImageFormat::Jpeg)
    ) {
        bail!("Only PNG and JPEG attachments are previewed");
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    // Decode with limits, discard metadata/animation, and send a bounded static preview to the webview.
    let preview = reader.decode()?.thumbnail(1600, 1600);
    let mut output = Cursor::new(Vec::new());
    preview.write_to(&mut output, image::ImageFormat::Png)?;
    let output = output.into_inner();
    if output.len() > MAX_IMAGE_BYTES {
        bail!("Image preview exceeds the 8 MB limit");
    }
    Ok(format!("data:image/png;base64,{}", STANDARD.encode(output)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_active_image_formats() {
        assert!(image_preview(
            b"<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>".to_vec()
        )
        .is_err());
    }
    #[test]
    fn generates_a_static_preview_from_a_small_png() {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1, 1)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        assert!(image_preview(bytes.into_inner())
            .unwrap()
            .starts_with("data:image/png;base64,"));
    }
}
