use serde::Serialize;

use crate::{
    models::{
        AdminDevice, AdminIssuerToken, AdminUser, Channel, CreateIssuerTokenResponse,
        CreatedDecisionRequest, DecisionRequest, EnrollmentCodeResponse, User, UserDevice,
    },
    views::RequestDecisionView,
};

#[derive(Debug, Serialize)]
pub(super) struct HealthResponse {
    ok: bool,
    service: &'static str,
}

impl HealthResponse {
    pub(super) fn nod() -> Self {
        Self {
            ok: true,
            service: "nod",
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct OkResponse {
    ok: bool,
}

impl OkResponse {
    pub(super) fn ok() -> Self {
        Self { ok: true }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct ChannelsResponse {
    pub(super) channels: Vec<Channel>,
}

#[derive(Debug, Serialize)]
pub(super) struct ChannelResponse {
    pub(super) channel: Channel,
}

#[derive(Debug, Serialize)]
pub(super) struct AdminUsersResponse {
    pub(super) users: Vec<AdminUser>,
}

#[derive(Debug, Serialize)]
pub(super) struct UserResponse {
    pub(super) user: User,
}

#[derive(Debug, Serialize)]
pub(super) struct UserDevicesResponse {
    pub(super) devices: Vec<UserDevice>,
}

#[derive(Debug, Serialize)]
pub(super) struct UserDeviceResponse {
    pub(super) device: UserDevice,
}

#[derive(Debug, Serialize)]
pub(super) struct AdminDevicesResponse {
    pub(super) devices: Vec<AdminDevice>,
}

#[derive(Debug, Serialize)]
pub(super) struct AdminIssuerTokensResponse {
    pub(super) tokens: Vec<AdminIssuerToken>,
}

pub(super) type EnrollmentResponse = EnrollmentCodeResponse;
pub(super) type IssuerTokenResponse = CreateIssuerTokenResponse;

#[derive(Debug, Serialize)]
pub(super) struct CreateRequestResponse {
    request_id: String,
    deduped: bool,
    request: nod_proto::Request,
}

impl CreateRequestResponse {
    pub(super) fn from_created_request(response: &CreatedDecisionRequest) -> Self {
        Self {
            request_id: response.request_id.clone(),
            deduped: response.deduped,
            request: response.request.to_wire(),
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct RequestsResponse {
    requests: Vec<nod_proto::Request>,
    next_cursor: Option<String>,
}

impl RequestsResponse {
    pub(super) fn from_page(requests: Vec<DecisionRequest>, next_cursor: Option<String>) -> Self {
        Self {
            requests: requests.iter().map(DecisionRequest::to_wire).collect(),
            next_cursor,
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct RequestResponse {
    request: nod_proto::Request,
}

impl RequestResponse {
    pub(super) fn from_request(request: &DecisionRequest) -> Self {
        Self {
            request: request.to_wire(),
        }
    }
}

pub(super) type RequestDecisionResponse = RequestDecisionView;
