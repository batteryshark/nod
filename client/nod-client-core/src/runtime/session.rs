use anyhow::{anyhow, Result};

use crate::{
    api::NodApi,
    models::{DecisionSignature, Request, ServerProfile},
    signing::{
        build_decision_signature, DecisionSigningRequest, DeviceSigner, ForeignDeviceSigner,
    },
};

use super::{snapshot::SnapshotContext, NodClientRuntime, SignerBackend, SubmitOptionParams};

impl NodClientRuntime {
    pub(super) async fn api(&self) -> Result<NodApi> {
        self.api_for_profile(&self.selected_server_profile().await?)
            .await
    }

    pub(super) async fn api_for_profile(&self, profile: &ServerProfile) -> Result<NodApi> {
        let persisted = self.persisted.lock().await;
        let token = self.store.load_token(&persisted, profile.credential_id())
            .ok_or_else(|| anyhow!("Device credentials are unavailable for {}. Unlock the credential store or enroll again.", profile.name))?;
        NodApi::with_client(
            &profile.base_url_string,
            Some(token),
            self.http_client.clone(),
        )
    }

    pub(super) async fn snapshot_context(&self) -> Result<SnapshotContext> {
        let profile = self.selected_server_profile().await?;
        Ok(SnapshotContext {
            api: self.api_for_profile(&profile).await?,
            server_id: profile.id,
            reducer: self.reducer.clone(),
            tx: self.tx.clone(),
            lock: self.snapshot_lock.clone(),
        })
    }

    pub(super) async fn decision_signature(
        &self,
        profile: &ServerProfile,
        request: &Request,
        option: &SubmitOptionParams,
    ) -> Result<Option<DecisionSignature>> {
        let signer = self.device_signer_for(profile).await?
            .ok_or_else(|| anyhow!("The device signing key is unavailable. Unlock the credential store or enroll again."))?;
        let user_id = profile
            .user_id
            .as_deref()
            .ok_or_else(|| anyhow!("server profile is missing user identity"))?;
        let device_id = profile
            .device_id
            .as_deref()
            .ok_or_else(|| anyhow!("server profile is missing device identity"))?;
        build_decision_signature(
            signer.as_ref(),
            DecisionSigningRequest {
                request,
                option_id: &option.option_id,
                text: option.text.as_deref(),
                user_id,
                device_id,
            },
        )
        .map(Some)
    }

    pub(super) async fn device_signer_for(
        &self,
        profile: &ServerProfile,
    ) -> Result<Option<Box<dyn DeviceSigner>>> {
        match self.signer_backend() {
            SignerBackend::Software => {
                let persisted = self.persisted.lock().await;
                Ok(self
                    .store
                    .load_signing_key(&persisted, profile.credential_id())
                    .map(|key| Box::new(key) as Box<dyn DeviceSigner>))
            }
            SignerBackend::Foreign(backend) => {
                let Some(key) = backend.signing_key(profile.credential_id())? else {
                    return Ok(None);
                };
                Ok(Some(Box::new(ForeignDeviceSigner {
                    backend: backend.clone(),
                    profile_id: profile.credential_id().to_string(),
                    key,
                }) as Box<dyn DeviceSigner>))
            }
        }
    }

    pub(super) async fn selected_server_profile(&self) -> Result<ServerProfile> {
        self.reducer
            .lock()
            .await
            .selected_server()
            .cloned()
            .ok_or_else(|| anyhow!("no selected server"))
    }

    pub(super) async fn server_profile(&self, server_id: &str) -> Result<ServerProfile> {
        self.persisted
            .lock()
            .await
            .servers
            .iter()
            .find(|profile| {
                profile.id == server_id || profile.credential_id.as_deref() == Some(server_id)
            })
            .cloned()
            .ok_or_else(|| {
                anyhow!("This notification belongs to a server that is no longer enrolled.")
            })
    }
}
