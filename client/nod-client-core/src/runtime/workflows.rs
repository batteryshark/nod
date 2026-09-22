use anyhow::{anyhow, Result};

use crate::{
    api::{
        display_name_for, normalize_base_url, profile_id_for, EnrollDeviceRequest,
        SubmitOptionRequest,
    },
    models::{
        ClientState, DevicePlatform, DeviceSigningKey, Request, RequestStatus, ServerProfile,
        UserDevice,
    },
    signing::{DeviceSigner, StoredSigningKey},
};

use super::{
    ChannelParams, DeviceNotificationPreferenceParams, EnrollParams, NodClientMessage,
    NodClientRuntime, OpenRequestParams, RegisterPushTokenParams, RenameDeviceParams,
    RevokeDeviceParams, SelectRequestParams, SetSubscriptionParams, SignerBackend,
    SubmitOptionParams, SubmitRequestOptionParams,
};

impl NodClientRuntime {
    pub async fn enroll(&mut self, params: EnrollParams) -> Result<ClientState> {
        let normalized_url = normalize_base_url(&params.base_url);
        let profile_id = profile_id_for(&normalized_url);
        // Provision the device key from the active backend: a software key the
        // store will persist, or a Secure Enclave key the host already holds.
        let (device_signing_key, software_key) = self.provision_device_signing_key(&profile_id)?;
        let api = crate::api::NodApi::with_client(&normalized_url, None, self.http_client.clone())?;
        let response = api
            .enroll(EnrollDeviceRequest {
                code: &params.code.trim().to_ascii_uppercase(),
                device_name: params.device_name.trim(),
                platform: params
                    .platform
                    .unwrap_or_else(DevicePlatform::current_desktop),
                native_app_id: params.native_app_id.as_deref(),
                push_provider: params.push_provider.as_deref(),
                push_token: params.push_token.as_deref(),
                signing_key: Some(&device_signing_key),
                attestation: params.attestation.as_ref(),
            })
            .await?;
        let profile = ServerProfile {
            id: profile_id.clone(),
            credential_id: None,
            name: display_name_for(&normalized_url),
            base_url_string: normalized_url,
            device_name: params.device_name.trim().to_string(),
            device_id: Some(response.device_id.clone()),
            user_id: Some(response.user_id.clone()),
            user_name: Some(response.user_name.clone()),
        };

        {
            let mut persisted = self.persisted.lock().await;
            self.store
                .save_token(&mut persisted, &profile_id, &response.token)
                .await?;
            // Only the software backend persists a private key; the Secure
            // Enclave key stays in the host's hardware, never in the store.
            if let Some(software_key) = &software_key {
                self.store
                    .save_signing_key(&mut persisted, &profile_id, software_key)
                    .await?;
            }
            persisted.selected_server_id = Some(profile_id.clone());
            persisted.notification_sound = params
                .notification_sound
                .unwrap_or_else(|| persisted.notification_sound.clone());
            upsert_profile(&mut persisted.servers, profile.clone());
            self.store.save(persisted.clone()).await?;
        }

        self.disconnect_sync().await;
        {
            let mut reducer = self.reducer.lock().await;
            reducer.upsert_server(profile);
            reducer.set_selected_server(profile_id);
            reducer.state.channels = response.channels;
            reducer.state.devices = response.devices;
            reducer.state.is_registered = true;
            reducer.set_notification_delivery_mode(response.notification_delivery.mode);
        }

        match self.refresh().await {
            Ok(state) => Ok(state),
            Err(error) => {
                // Enrollment already consumed the code and persisted credentials.
                // A failed inbox fetch must not invite a second enrollment attempt.
                self.reducer.lock().await.set_error(format!(
                    "Enrollment completed. Inbox sync will retry: {error}"
                ));
                self.emit_state().await;
                Ok(self.state().await)
            }
        }
    }

    pub async fn select_server(&mut self, server_id: String) -> Result<ClientState> {
        {
            let mut persisted = self.persisted.lock().await;
            if !persisted
                .servers
                .iter()
                .any(|server| server.id == server_id)
            {
                return Err(anyhow!("unknown server: {server_id}"));
            }
            persisted.selected_server_id = Some(server_id.clone());
            self.store.save(persisted.clone()).await?;
        }

        self.disconnect_sync().await;
        self.reducer.lock().await.set_selected_server(server_id);
        self.connect_sync().await?;
        Ok(self.state().await)
    }

    pub async fn forget_server(&mut self, server_id: &str) -> Result<ClientState> {
        let profile = self.server_profile(server_id).await?;
        let server_id = profile.id.as_str();
        let was_selected = self.state().await.selected_server_id.as_deref() == Some(server_id);
        if was_selected {
            self.disconnect_sync().await;
        }
        // Drop the host-held hardware key (no-op for the software backend, whose
        // key lives in the store and is cleared below). Done outside the store
        // lock so the foreign callback isn't held across the mutex.
        if let SignerBackend::Foreign(backend) = self.signer_backend() {
            backend.remove(profile.credential_id())?;
        }
        {
            let mut persisted = self.persisted.lock().await;
            persisted.servers.retain(|server| server.id != server_id);
            self.store
                .delete_token(&mut persisted, profile.credential_id())
                .await?;
            self.store
                .delete_signing_key(&mut persisted, profile.credential_id())
                .await?;
            if persisted.selected_server_id.as_deref() == Some(server_id) {
                persisted.selected_server_id =
                    persisted.servers.first().map(|server| server.id.clone());
            }
            self.store.save(persisted.clone()).await?;
        }

        self.reducer.lock().await.remove_server(server_id);
        if was_selected && self.state().await.is_registered {
            self.connect_sync().await?;
        }
        self.emit_state().await;
        Ok(self.state().await)
    }

    pub async fn refresh(&mut self) -> Result<ClientState> {
        self.snapshot_context().await?.refresh().await
    }

    pub async fn query_history(
        &self,
        params: super::QueryHistoryParams,
    ) -> Result<crate::models::RequestsResponse> {
        let profile = match params.server_id.as_deref() {
            Some(id) => self.server_profile(id).await?,
            None => self.selected_server_profile().await?,
        };
        self.api_for_profile(&profile)
            .await?
            .query_history(&params)
            .await
    }

    pub async fn open_request(&mut self, params: OpenRequestParams) -> Result<ClientState> {
        let profile = self.server_profile(&params.server_id).await?;
        let request = self
            .api_for_profile(&profile)
            .await?
            .get_request(&params.request_id)
            .await?;
        if self.state().await.selected_server_id.as_deref() != Some(&profile.id) {
            self.select_server(profile.id).await?;
        }
        {
            let mut reducer = self.reducer.lock().await;
            reducer.apply_request_update(request.clone());
            reducer.select_channel(Some(request.channel_id));
            reducer.state.selected_request_id = Some(request.id);
        }
        self.emit_state().await;
        Ok(self.state().await)
    }

    pub async fn submit_option(&mut self, params: SubmitOptionParams) -> Result<Request> {
        let profile = self.selected_server_profile().await?;
        self.submit_request_option(SubmitRequestOptionParams {
            server_id: profile.id,
            request_id: params.request_id,
            option_id: params.option_id,
            text: params.text,
        })
        .await
    }

    pub async fn submit_request_option(
        &mut self,
        params: SubmitRequestOptionParams,
    ) -> Result<Request> {
        let profile = self.server_profile(&params.server_id).await?;
        let api = self.api_for_profile(&profile).await?;
        let authoritative = api.get_request(&params.request_id).await?;
        if authoritative.status != RequestStatus::Pending {
            self.apply_targeted_request(&profile.id, authoritative)
                .await;
            return Err(anyhow!(
                "This request has already been handled. Refresh to see its outcome."
            ));
        }
        let option = SubmitOptionParams {
            request_id: params.request_id,
            option_id: params.option_id,
            text: params
                .text
                .as_deref()
                .and_then(trimmed_text)
                .map(str::to_string),
        };
        let signature = self
            .decision_signature(&profile, &authoritative, &option)
            .await?;
        let result = api
            .submit_option(SubmitOptionRequest {
                request_id: &option.request_id,
                option_id: &option.option_id,
                text: option.text.as_deref(),
                signature: signature.as_ref(),
            })
            .await;
        match result {
            Ok(request) => {
                self.apply_targeted_request(&profile.id, request.clone())
                    .await;
                Ok(request)
            }
            Err(error) => {
                // The response may have been lost after commit. Re-read without
                // resubmitting so the user can distinguish an uncertain outcome.
                if let Ok(current) = api.get_request(&option.request_id).await {
                    self.apply_targeted_request(&profile.id, current).await;
                }
                Err(error)
            }
        }
    }

    async fn apply_targeted_request(&self, server_id: &str, request: Request) {
        if self.state().await.selected_server_id.as_deref() == Some(server_id) {
            self.reducer
                .lock()
                .await
                .apply_request_update(request.clone());
            self.emit_state().await;
        }
        if request.status != RequestStatus::Pending {
            self.emit_message(NodClientMessage::NotificationRemoved {
                server_id: server_id.to_string(),
                request_id: request.id,
            })
            .await;
        }
    }

    pub async fn clear_channel(&mut self, params: ChannelParams) -> Result<ClientState> {
        self.api().await?.clear_channel(&params.channel_id).await?;
        self.refresh().await
    }

    pub async fn set_subscription(&mut self, params: SetSubscriptionParams) -> Result<ClientState> {
        self.api()
            .await?
            .set_subscription(&params.channel_id, params.subscribed)
            .await?;
        self.refresh().await
    }

    pub async fn set_device_notification_preferences(
        &mut self,
        params: DeviceNotificationPreferenceParams,
    ) -> Result<ClientState> {
        let profile = match params.server_id {
            Some(server_id) => self.server_profile(&server_id).await?,
            None => self.selected_server_profile().await?,
        };
        self.api_for_profile(&profile)
            .await?
            .set_device_notification_preferences(&params.preferences)
            .await?;
        if self.state().await.selected_server_id.as_deref() == Some(&profile.id) {
            {
                let mut reducer = self.reducer.lock().await;
                if let Some(device) = reducer
                    .state
                    .devices
                    .iter_mut()
                    .find(|device| Some(device.id.as_str()) == profile.device_id.as_deref())
                {
                    device.notification_preferences = params.preferences;
                }
            }
            self.emit_state().await;
            self.refresh().await
        } else {
            Ok(self.state().await)
        }
    }

    pub async fn set_notification_preference(
        &mut self,
        notification_sound: &str,
    ) -> Result<ClientState> {
        self.api()
            .await?
            .set_notification_sound(notification_sound)
            .await?;
        {
            let mut persisted = self.persisted.lock().await;
            persisted.notification_sound = notification_sound.to_string();
            self.store.save(persisted.clone()).await?;
        }
        self.reducer.lock().await.state.notification_sound = notification_sound.to_string();
        self.emit_state().await;
        Ok(self.state().await)
    }

    /// Register/refresh the APNs push token across every enrolled server (the
    /// same token applies to all). Mirrors NodKit's `registerPushToken`.
    pub async fn register_push_token(
        &mut self,
        params: RegisterPushTokenParams,
    ) -> Result<ClientState> {
        let servers = { self.persisted.lock().await.servers.clone() };
        let mut failures = Vec::new();
        for server in &servers {
            let result = async {
                self.api_for_profile(server)
                    .await?
                    .update_push_token(&params.provider, &params.native_app_id, &params.token)
                    .await
            }
            .await;
            if let Err(error) = result {
                failures.push(format!("{}: {error}", server.name));
            }
        }
        if !failures.is_empty() {
            return Err(anyhow!(
                "Push registration updated on {} of {} servers. Retry the failed servers: {}",
                servers.len() - failures.len(),
                servers.len(),
                failures.join("; ")
            ));
        }
        self.refresh().await
    }

    pub async fn list_devices(&mut self) -> Result<Vec<UserDevice>> {
        let devices = self.api().await?.devices().await?;
        self.reducer.lock().await.state.devices = devices.clone();
        self.emit_state().await;
        Ok(devices)
    }

    pub async fn rename_device(&mut self, params: RenameDeviceParams) -> Result<UserDevice> {
        let device = self
            .api()
            .await?
            .rename_device(&params.device_id, &params.name)
            .await?;
        let mut reducer = self.reducer.lock().await;
        if let Some(existing) = reducer
            .state
            .devices
            .iter_mut()
            .find(|existing| existing.id == device.id)
        {
            *existing = device.clone();
        }
        drop(reducer);
        self.emit_state().await;
        Ok(device)
    }

    pub async fn revoke_device(&mut self, params: RevokeDeviceParams) -> Result<ClientState> {
        self.api().await?.revoke_device(&params.device_id).await?;
        self.refresh_or_forget_current(&params.device_id).await
    }

    pub async fn select_channel(&mut self, params: ChannelParams) -> Result<ClientState> {
        if !self
            .state()
            .await
            .channels
            .iter()
            .any(|channel| channel.id == params.channel_id && channel.subscribed)
        {
            return Err(anyhow!("channel is unavailable or unsubscribed"));
        }
        self.reducer
            .lock()
            .await
            .select_channel(Some(params.channel_id));
        self.emit_state().await;
        Ok(self.state().await)
    }

    pub async fn select_all_channels(&mut self) -> Result<ClientState> {
        self.reducer.lock().await.select_channel(None);
        self.emit_state().await;
        Ok(self.state().await)
    }

    pub async fn select_request(&mut self, params: SelectRequestParams) -> Result<ClientState> {
        self.reducer.lock().await.state.selected_request_id = Some(params.request_id);
        self.emit_state().await;
        Ok(self.state().await)
    }

    async fn refresh_or_forget_current(&mut self, revoked_device_id: &str) -> Result<ClientState> {
        let current_was_revoked = self
            .reducer
            .lock()
            .await
            .selected_server()
            .and_then(|server| server.device_id.as_deref())
            == Some(revoked_device_id);
        if current_was_revoked {
            let server_id = {
                self.reducer
                    .lock()
                    .await
                    .selected_server()
                    .map(|server| server.id.clone())
            };
            if let Some(server_id) = server_id {
                return self.forget_server(&server_id).await;
            }
        }
        self.refresh().await
    }

    /// Provision the device signing key for a newly enrolling profile. Returns
    /// the public `DeviceSigningKey` to register with the server, plus the
    /// software key to persist (`Some` for the software backend, `None` for the
    /// Secure Enclave — its key never leaves the host's hardware).
    fn provision_device_signing_key(
        &self,
        profile_id: &str,
    ) -> Result<(DeviceSigningKey, Option<StoredSigningKey>)> {
        match self.signer_backend() {
            SignerBackend::Software => {
                let key = StoredSigningKey::generate();
                let device_signing_key = key.device_signing_key()?;
                Ok((device_signing_key, Some(key)))
            }
            SignerBackend::Foreign(backend) => {
                let provisioned = backend.provision(profile_id)?;
                let device_signing_key = DeviceSigningKey {
                    key_id: provisioned.key_id,
                    algorithm: nod_proto::DECISION_SIGNING_ALGORITHM.to_string(),
                    public_key: provisioned.public_key,
                };
                Ok((device_signing_key, None))
            }
        }
    }
}

fn upsert_profile(profiles: &mut Vec<ServerProfile>, profile: ServerProfile) {
    if let Some(existing) = profiles
        .iter_mut()
        .find(|existing| existing.id == profile.id)
    {
        *existing = profile;
    } else {
        profiles.push(profile);
    }
}

fn trimmed_text(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}
