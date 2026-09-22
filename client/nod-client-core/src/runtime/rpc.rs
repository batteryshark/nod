use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::models::DevicePlatform;

use super::NodClientRuntime;

#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    #[serde(default)]
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct RpcResponse {
    pub id: Value,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl RpcResponse {
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(id: Value, error: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(error.into()),
        }
    }
}

impl NodClientRuntime {
    pub async fn handle_rpc(&mut self, request: RpcRequest) -> RpcResponse {
        match self.handle_method(&request.method, request.params).await {
            Ok(result) => RpcResponse::success(request.id, result),
            Err(error) => {
                let message = error.to_string();
                self.reducer.lock().await.set_error(message.clone());
                self.emit_state().await;
                RpcResponse::failure(request.id, message)
            }
        }
    }

    pub async fn handle_method(&mut self, method: &str, params: Value) -> Result<Value> {
        match method {
            "state" => Ok(json!(self.state().await)),
            "enroll" => {
                let params: EnrollParams = serde_json::from_value(params)?;
                Ok(json!(self.enroll(params).await?))
            }
            "select_server" => {
                let params: SelectServerParams = serde_json::from_value(params)?;
                Ok(json!(self.select_server(params.server_id).await?))
            }
            "forget_server" => {
                let params: SelectServerParams = serde_json::from_value(params)?;
                Ok(json!(self.forget_server(&params.server_id).await?))
            }
            "refresh" => Ok(json!(self.refresh().await?)),
            "open_request" => Ok(json!(
                self.open_request(serde_json::from_value(params)?).await?
            )),
            "submit_request_option" => Ok(
                json!({"request": self.submit_request_option(serde_json::from_value(params)?).await?}),
            ),
            "select_all_channels" => Ok(json!(self.select_all_channels().await?)),
            "query_history" => Ok(json!(
                self.query_history(serde_json::from_value(params)?).await?
            )),
            "submit_option" => {
                let params: SubmitOptionParams = serde_json::from_value(params)?;
                Ok(json!({ "request": self.submit_option(params).await? }))
            }
            "clear_channel" => {
                let params: ChannelParams = serde_json::from_value(params)?;
                Ok(json!(self.clear_channel(params).await?))
            }
            "set_subscription" => {
                let params: SetSubscriptionParams = serde_json::from_value(params)?;
                Ok(json!(self.set_subscription(params).await?))
            }
            "set_device_notification_preferences" => Ok(json!(
                self.set_device_notification_preferences(serde_json::from_value(params)?)
                    .await?
            )),
            "set_notification_preference" => {
                let params: NotificationPreferenceParams = serde_json::from_value(params)?;
                Ok(json!(
                    self.set_notification_preference(&params.notification_sound)
                        .await?
                ))
            }
            "register_push_token" => {
                let params: RegisterPushTokenParams = serde_json::from_value(params)?;
                Ok(json!(self.register_push_token(params).await?))
            }
            "list_devices" => Ok(json!({ "devices": self.list_devices().await? })),
            "rename_device" => {
                let params: RenameDeviceParams = serde_json::from_value(params)?;
                Ok(json!({ "device": self.rename_device(params).await? }))
            }
            "revoke_device" => {
                let params: RevokeDeviceParams = serde_json::from_value(params)?;
                Ok(json!(self.revoke_device(params).await?))
            }
            "connect_sync" => {
                self.connect_sync().await?;
                Ok(json!({ "connected": true }))
            }
            "disconnect_sync" => {
                self.disconnect_sync().await;
                Ok(json!({ "connected": false }))
            }
            "select_channel" => {
                let params: ChannelParams = serde_json::from_value(params)?;
                Ok(json!(self.select_channel(params).await?))
            }
            "select_request" => {
                let params: SelectRequestParams = serde_json::from_value(params)?;
                Ok(json!(self.select_request(params).await?))
            }
            _ => Err(anyhow!("unknown method: {method}")),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EnrollParams {
    pub base_url: String,
    pub device_name: String,
    pub code: String,
    #[serde(default)]
    pub notification_sound: Option<String>,
    #[serde(default)]
    pub platform: Option<DevicePlatform>,
    // Native enrollment hardening provided by the host (Apple): push delivery
    // and App Attest. nod-client-core forwards them to the server unchanged.
    #[serde(default)]
    pub native_app_id: Option<String>,
    #[serde(default)]
    pub push_provider: Option<String>,
    #[serde(default)]
    pub push_token: Option<String>,
    #[serde(default)]
    pub attestation: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SelectServerParams {
    pub server_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SubmitOptionParams {
    pub request_id: String,
    pub option_id: String,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenRequestParams {
    pub server_id: String,
    pub request_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct QueryHistoryParams {
    #[serde(default)]
    pub server_id: Option<String>,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SubmitRequestOptionParams {
    pub server_id: String,
    pub request_id: String,
    pub option_id: String,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChannelParams {
    pub channel_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SetSubscriptionParams {
    pub channel_id: String,
    pub subscribed: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeviceNotificationPreferenceParams {
    #[serde(default)]
    pub server_id: Option<String>,
    #[serde(flatten)]
    pub preferences: crate::models::DeviceNotificationPreferences,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NotificationPreferenceParams {
    pub notification_sound: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RegisterPushTokenParams {
    pub provider: String,
    pub native_app_id: String,
    pub token: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RenameDeviceParams {
    pub device_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RevokeDeviceParams {
    pub device_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SelectRequestParams {
    pub request_id: String,
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::*;

    #[test]
    fn success_response_serializes_result_without_error() {
        let response = RpcResponse::success(json!("request-1"), json!({ "connected": true }));

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            json!({
                "id": "request-1",
                "ok": true,
                "result": { "connected": true }
            })
        );
    }

    #[test]
    fn failure_response_serializes_error_without_result() {
        let response = RpcResponse::failure(Value::Null, "bad request");

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            json!({
                "id": null,
                "ok": false,
                "error": "bad request"
            })
        );
    }
}
