use anyhow::Result;
use async_trait::async_trait;
use nod_client_core::{
    models::{ClientState, Request, UserDevice},
    ChannelParams, NodClientRuntime, NotificationPreferenceParams, RenameDeviceParams,
    RevokeDeviceParams, SelectRequestParams, SelectServerParams, SetSubscriptionParams,
    SubmitOptionParams,
};

use super::RuntimePort;

#[async_trait]
impl RuntimePort for NodClientRuntime {
    async fn enroll(&mut self, params: nod_client_core::EnrollParams) -> Result<ClientState> {
        self.enroll(params).await
    }

    async fn refresh(&mut self) -> Result<ClientState> {
        self.refresh().await
    }

    async fn connect_sync(&mut self) -> Result<()> {
        self.connect_sync().await
    }

    async fn select_server(&mut self, params: SelectServerParams) -> Result<ClientState> {
        self.select_server(params.server_id).await
    }

    async fn forget_server(&mut self, params: SelectServerParams) -> Result<ClientState> {
        self.forget_server(&params.server_id).await
    }

    async fn select_channel(&mut self, params: ChannelParams) -> Result<ClientState> {
        self.select_channel(params).await
    }

    async fn query_history(
        &mut self,
        params: nod_client_core::QueryHistoryParams,
    ) -> Result<nod_client_core::models::RequestsResponse> {
        NodClientRuntime::query_history(self, params).await
    }

    async fn select_all_channels(&mut self) -> Result<ClientState> {
        self.select_all_channels().await
    }

    async fn select_request(&mut self, params: SelectRequestParams) -> Result<ClientState> {
        self.select_request(params).await
    }

    async fn submit_option(&mut self, params: SubmitOptionParams) -> Result<Request> {
        self.submit_option(params).await
    }

    async fn clear_channel(&mut self, params: ChannelParams) -> Result<ClientState> {
        self.clear_channel(params).await
    }

    async fn set_subscription(&mut self, params: SetSubscriptionParams) -> Result<ClientState> {
        self.set_subscription(params).await
    }

    async fn set_notification_preference(
        &mut self,
        params: NotificationPreferenceParams,
    ) -> Result<ClientState> {
        self.set_notification_preference(&params.notification_sound)
            .await
    }

    async fn list_devices(&mut self) -> Result<Vec<UserDevice>> {
        self.list_devices().await
    }

    async fn rename_device(&mut self, params: RenameDeviceParams) -> Result<UserDevice> {
        self.rename_device(params).await
    }

    async fn revoke_device(&mut self, params: RevokeDeviceParams) -> Result<ClientState> {
        self.revoke_device(params).await
    }
}
