use nod_client_core::{models::Request, OpenRequestParams, SubmitRequestOptionParams};

use super::{options::value_unsupported, NotificationActivation};

// Small seam for proving queued OS actions cannot turn unsupported cards into writes.
pub(crate) trait ActivationRuntime {
    fn read(
        &mut self,
        params: OpenRequestParams,
    ) -> impl std::future::Future<Output = anyhow::Result<Request>> + Send;
    fn open(
        &mut self,
        params: OpenRequestParams,
    ) -> impl std::future::Future<Output = anyhow::Result<()>> + Send;
    fn submit(
        &mut self,
        params: SubmitRequestOptionParams,
    ) -> impl std::future::Future<Output = anyhow::Result<()>> + Send;
}

pub(crate) async fn activate(
    runtime: &mut impl ActivationRuntime,
    activation: NotificationActivation,
) -> anyhow::Result<()> {
    match activation {
        NotificationActivation::Open {
            server_id,
            request_id,
        } => {
            if let Some(request_id) = request_id {
                runtime
                    .open(OpenRequestParams {
                        server_id,
                        request_id,
                    })
                    .await?;
            }
        }
        NotificationActivation::Submit {
            server_id,
            request_id,
            option_id,
        } => {
            let params = OpenRequestParams {
                server_id: server_id.clone(),
                request_id: request_id.clone(),
            };
            // Fresh, read-only lookup. Any failure stops here; never retry mutation.
            let current = runtime.read(params.clone()).await?;
            if value_unsupported(&current) {
                runtime.open(params).await?;
            } else {
                runtime
                    .submit(SubmitRequestOptionParams {
                        server_id,
                        request_id,
                        option_id,
                        text: None,
                        answer_value: None,
                        expected_request_digest: None,
                    })
                    .await?;
            }
        }
    }
    Ok(())
}
