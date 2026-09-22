// Frontend wire + view types. These mirror the Rust serde payloads (snake_case),
// but intentionally encode the *practical* contract the Tauri backend guarantees:
// fields the backend always populates are required here, so the UI reads them
// without defensive `?.` noise.
//
// The canonical definitions live in Rust (nod-proto + nod-client-core) under
// `#[typeshare]`; `scripts/generate-types.sh` projects them to a throwaway
// `generated.ts` you can diff against these to catch drift. (We don't import the
// generated types directly: typeshare turns every `#[serde(default)]` field into
// an optional, which the backend never actually omits.)
export type DevicePlatform =
  | "ios"
  | "macos"
  | "watchos"
  | "windows"
  | "linux"
  | "unknown";

export type NotificationDeliveryMode = "push" | "websocket";
export type DecisionResolution = "shared" | "per_user";
export type RequestStatus = "pending" | "resolved" | "expired" | "cancelled";
export type OptionKind =
  | "approve"
  | "approve_with_text"
  | "reject"
  | "reject_with_text"
  | "dismiss"
  | "open"
  | "custom";

export interface ServerProfile {
  id: string;
  name: string;
  base_url_string: string;
  credential_id?: string | null;
  device_name: string;
  device_id?: string | null;
  user_id?: string | null;
  user_name?: string | null;
}

export interface User {
  id: string;
  name: string;
  created_at: string;
  updated_at: string;
}

export type DeviceAttestationStatus = "verified" | "failed";

// The public attestation summary the server reports for a device.
export interface DeviceAttestationSummary {
  provider: string;
  status: DeviceAttestationStatus;
  key_id?: string | null;
  team_id?: string | null;
  bundle_id?: string | null;
  environment?: string | null;
  verified_at?: string | null;
  failure_reason?: string | null;
}

export interface UserDevice {
  id: string;
  user_id: string;
  name: string;
  platform: DevicePlatform;
  native_app_id?: string | null;
  push_provider?: string | null;
  has_push_token: boolean;
  has_signing_key: boolean;
  attestation?: DeviceAttestationSummary | null;
  notification_sound: string;
  notification_preferences?: DeviceNotificationPreferences;
  last_seen_at: string;
  created_at: string;
  is_current: boolean;
}

export interface Channel {
  id: string;
  name: string;
  emoji: string;
  subscribed: boolean;
  created_at: string;
}

export interface RequestField {
  label: string;
  value: string;
  style?: string | null;
}

export interface RequestLink {
  label: string;
  url: string;
}

export interface RequestOption {
  id: string;
  label: string;
  kind: OptionKind;
  style: string;
  requires_text: boolean;
  text_placeholder?: string | null;
  destructive: boolean;
  foreground: boolean;
}

// The signature record the server stores and republishes on a decision:
// the client-submitted signature plus the server's verification verdict.
export interface DecisionSignatureRecord {
  key_id: string;
  algorithm: string;
  nonce: string;
  signed_at: string;
  request_digest: string;
  signing_payload: string;
  signature: string;
  verified: boolean;
  public_key?: string | null;
}

export interface Decision {
  request_id: string;
  option_id: string;
  option_kind: OptionKind;
  option_label: string;
  text?: string | null;
  actor_user_id?: string | null;
  actor_device_id?: string | null;
  signature?: DecisionSignatureRecord | null;
  resolved_at: string;
}

export interface UserDecision {
  user_id: string;
  decision: Decision;
}

// Rust calls this `Request`; the frontend renames it because `Request` is
// already the Fetch API's global type — importing the wire name shadows it
// and invites silent type confusion in browser code.
export interface NodRequest {
  id: string;
  request_id: string;
  channel_id: string;
  recipients: string[];
  decision_resolution: DecisionResolution;
  title: string;
  summary: string;
  body_markdown: string;
  fields: RequestField[];
  links: RequestLink[];
  image_url?: string | null;
  notification: RequestNotification;
  dedupe_key?: string | null;
  expires_at?: string | null;
  status: RequestStatus;
  created_at: string;
  updated_at: string;
  resolved_at?: string | null;
  decision?: Decision | null;
  decisions: UserDecision[];
  callback_url?: string | null;
  options: RequestOption[];
  request_digest?: string | null;
  signing?: RequestSigningContext | null;
}

export interface RequestNotification {
  redact: boolean;
  title?: string | null;
  body?: string | null;
}

export interface ClientState {
  servers: ServerProfile[];
  selected_server_id?: string | null;
  current_user?: User | null;
  devices: UserDevice[];
  channels: Channel[];
  pending_counts_by_channel: Record<string, number>;
  requests: NodRequest[];
  selected_channel_id?: string | null;
  selected_request_id?: string | null;
  notification_sound: string;
  notification_delivery_mode: NotificationDeliveryMode;
  is_registered: boolean;
  is_sync_connected: boolean;
  sync_phase: SyncPhase;
  last_synced_at?: string | null;
  last_error?: string | null;
}

export interface RequestSigningContext {
  version: string;
  recipients_commitment: string;
  request_digest: string;
}

export type SyncPhase =
  | "offline"
  | "connecting"
  | "reconciling"
  | "current"
  | "revoked";

export interface DeviceNotificationPreferences {
  hide_content: boolean;
  muted_channels: string[];
  snoozed_until?: string | null;
}
