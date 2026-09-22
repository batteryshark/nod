use anyhow::Result;
use async_trait::async_trait;
use nod_client_core::{
    models::{ClientState, Request, UserDevice},
    ChannelParams, EnrollParams, NotificationPreferenceParams, RenameDeviceParams,
    RevokeDeviceParams, SelectRequestParams, SelectServerParams, SetSubscriptionParams,
    SubmitOptionParams,
};

/// The TUI's only command seam to `nod-client-core`: everything the UI asks
/// of the runtime — enrollment, sync, selection, decisions, device management
/// — goes through this trait, and no client or crypto logic lives in the TUI
/// itself. (Push traffic flows the other way, as `NodClientMessage`s on the
/// channel handed to `NodClientRuntime::new`.) Production binds this to
/// `NodClientRuntime` via `adapter.rs`; tests bind the in-memory
/// `FakeRuntime` from `test_support`.
#[async_trait]
pub(crate) trait RuntimePort {
    async fn enroll(&mut self, params: EnrollParams) -> Result<ClientState>;
    async fn refresh(&mut self) -> Result<ClientState>;
    async fn connect_sync(&mut self) -> Result<()>;
    async fn select_server(&mut self, params: SelectServerParams) -> Result<ClientState>;
    async fn forget_server(&mut self, params: SelectServerParams) -> Result<ClientState>;
    async fn select_channel(&mut self, params: ChannelParams) -> Result<ClientState>;
    async fn query_history(
        &mut self,
        params: nod_client_core::QueryHistoryParams,
    ) -> Result<nod_client_core::models::RequestsResponse>;
    async fn select_all_channels(&mut self) -> Result<ClientState>;
    async fn select_request(&mut self, params: SelectRequestParams) -> Result<ClientState>;
    async fn submit_option(&mut self, params: SubmitOptionParams) -> Result<Request>;
    async fn clear_channel(&mut self, params: ChannelParams) -> Result<ClientState>;
    async fn set_subscription(&mut self, params: SetSubscriptionParams) -> Result<ClientState>;
    async fn set_notification_preference(
        &mut self,
        params: NotificationPreferenceParams,
    ) -> Result<ClientState>;
    async fn list_devices(&mut self) -> Result<Vec<UserDevice>>;
    async fn rename_device(&mut self, params: RenameDeviceParams) -> Result<UserDevice>;
    async fn revoke_device(&mut self, params: RevokeDeviceParams) -> Result<ClientState>;
}
