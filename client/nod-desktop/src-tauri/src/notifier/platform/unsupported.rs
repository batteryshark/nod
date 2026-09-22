use nod_client_core::models::Request;

pub(crate) async fn show_notification(_request: &Request) -> anyhow::Result<()> {
    // Keep development builds portable while making unsupported runtime behavior explicit.
    anyhow::bail!("desktop notifications are only supported on Windows and Linux")
}

pub(crate) async fn remove_notification(_server_id: &str, _request_id: &str) -> anyhow::Result<()> {
    anyhow::bail!("desktop notification removal is only supported on Windows and Linux")
}
