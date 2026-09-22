use serde_json::json;

use crate::{
    db, device_attestation,
    error::ApiError,
    models::{
        Device, DeviceAttestationRecord, EnrollDeviceRequest, EnrollDeviceResponse,
        UpdateDevicePreferencesRequest, UpdatePushTokenRequest, UpdateSubscriptionRequest,
        UpdateUserDeviceRequest, UserDevice,
    },
    state::AppState,
    sync,
};

pub(crate) async fn enroll(
    state: &AppState,
    mut request: EnrollDeviceRequest,
) -> Result<EnrollDeviceResponse, ApiError> {
    let attestation = device_attestation::verify_enrollment_attestation(
        &state.config.device_attestation.apple_app_attest,
        &request,
    );
    apply_attested_native_app_id(&mut request, attestation.as_ref());
    let mut response =
        db::enroll_device(&state.pool, request, state.notification_delivery.clone()).await?;
    response.notification_delivery =
        state.delivery_for_device(&db::get_device(&state.pool, &response.device_id).await?);
    if let Some(attestation) = attestation {
        match db::record_device_attestation(&state.pool, &response.device_id, attestation).await {
            Ok(()) => {
                response.devices =
                    db::list_user_devices(&state.pool, &response.user_id, &response.device_id)
                        .await?;
            }
            Err(err) => {
                tracing::error!(
                    error = %err,
                    device_id = %response.device_id,
                    "failed to record device attestation"
                );
            }
        }
    }
    state
        .audit
        .record(
            "device.enrolled",
            &json!({ "device_id": response.device_id, "user_id": response.user_id }),
        )
        .await;
    let _ = state.sync.send(sync::targeted_envelope(
        "device_enrolled",
        json!({ "device_id": &response.device_id, "user_id": &response.user_id }),
        vec![response.user_id.clone()],
    ));
    Ok(response)
}

pub(crate) async fn rename_user_device(
    state: &AppState,
    current_device: &Device,
    device_id: &str,
    request: UpdateUserDeviceRequest,
) -> Result<UserDevice, ApiError> {
    let updated = db::rename_user_device(
        &state.pool,
        &current_device.user_id,
        device_id,
        &current_device.id,
        request,
    )
    .await?;
    state
        .audit
        .record(
            "device.renamed",
            &json!({ "device_id": &updated.id, "user_id": &updated.user_id }),
        )
        .await;
    let _ = state.sync.send(sync::targeted_envelope(
        "device_updated",
        json!({ "device": &updated }),
        vec![current_device.user_id.clone()],
    ));
    Ok(updated)
}

pub(crate) async fn revoke_user_device(
    state: &AppState,
    current_device: &Device,
    device_id: &str,
) -> Result<(), ApiError> {
    db::revoke_user_device(&state.pool, &current_device.user_id, device_id).await?;
    state
        .audit
        .record(
            "device.revoked",
            &json!({ "device_id": device_id, "user_id": &current_device.user_id }),
        )
        .await;
    let _ = state.sync.send(sync::targeted_envelope(
        "device_revoked",
        json!({ "device_id": device_id, "user_id": &current_device.user_id }),
        vec![current_device.user_id.clone()],
    ));
    Ok(())
}

pub(crate) async fn update_push_token(
    state: &AppState,
    device: &Device,
    request: UpdatePushTokenRequest,
) -> Result<(), ApiError> {
    let provider = request.provider.clone();
    let native_app_id = request.native_app_id.clone();
    db::update_push_token(&state.pool, &device.id, request).await?;
    let _ = state.sync.send(sync::targeted_envelope(
        "device_push_updated",
        json!({"device_id":device.id}),
        vec![device.user_id.clone()],
    ));
    state
        .audit
        .record(
            "device.push_token_updated",
            &json!({
                "device_id": device.id,
                "provider": provider,
                "native_app_id": native_app_id
            }),
        )
        .await;
    Ok(())
}

pub(crate) async fn update_preferences(
    state: &AppState,
    device: &Device,
    request: UpdateDevicePreferencesRequest,
) -> Result<(), ApiError> {
    db::update_device_preferences(&state.pool, &device.id, request).await?;
    state
        .audit
        .record(
            "device.preferences_updated",
            &json!({ "device_id": device.id }),
        )
        .await;
    Ok(())
}

pub(crate) async fn update_subscription(
    state: &AppState,
    device: &Device,
    channel_id: &str,
    request: UpdateSubscriptionRequest,
) -> Result<(), ApiError> {
    db::set_subscription(&state.pool, &device.id, channel_id, request.subscribed).await?;
    let envelope = sync::device_update(
        "subscription_updated",
        json!({ "device_id": device.id, "channel_id": channel_id, "subscribed": request.subscribed }),
    );
    let _ = state.sync.send(envelope);
    Ok(())
}

pub(crate) async fn clear_channel(
    state: &AppState,
    device: &Device,
    channel_id: &str,
) -> Result<(), ApiError> {
    db::clear_channel(&state.pool, &device.id, channel_id).await?;
    state
        .audit
        .record(
            "channel.cleared",
            &json!({ "device_id": device.id, "channel_id": channel_id }),
        )
        .await;
    let _ = state.sync.send(sync::device_update(
        "cleared",
        json!({ "device_id": device.id, "channel_id": channel_id }),
    ));
    Ok(())
}

fn apply_attested_native_app_id(
    request: &mut EnrollDeviceRequest,
    attestation: Option<&DeviceAttestationRecord>,
) {
    if request
        .native_app_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return;
    }
    request.native_app_id = attestation
        .and_then(|record| record.bundle_id.as_ref())
        .cloned();
}
