use super::PluginManager;
use anyhow::{bail, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const OBJECT_STORE_CAPABILITY: &str = "object-store.v1";

pub const PUT_REQUEST_OBJECT_METHOD: &str = "blobstore/put_request_object";
pub const GET_REQUEST_OBJECT_METHOD: &str = "blobstore/get_request_object";
pub const COMPLETE_REQUEST_METHOD: &str = "blobstore/complete_request";
pub const ABORT_REQUEST_METHOD: &str = "blobstore/abort_request";
pub const PUT_REQUEST_OBJECT_TOOL: &str = "put_request_object";
pub const GET_REQUEST_OBJECT_TOOL: &str = "get_request_object";
pub const COMPLETE_REQUEST_TOOL: &str = "complete_request";
pub const ABORT_REQUEST_TOOL: &str = "abort_request";

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct PutRequestObjectRequest {
    pub request_id: String,
    pub mime_type: String,
    #[serde(default)]
    pub file_name: Option<String>,
    pub bytes_base64: String,
    #[serde(default)]
    pub expires_in_secs: Option<u64>,
    #[serde(default)]
    pub uses_remaining: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct PutRequestObjectResponse {
    pub token: String,
    pub request_id: String,
    pub mime_type: String,
    #[serde(default)]
    pub file_name: Option<String>,
    pub size_bytes: u64,
    pub sha256_hex: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub uses_remaining: u32,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct GetRequestObjectRequest {
    pub token: String,
    #[serde(default)]
    pub request_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct GetRequestObjectResponse {
    pub token: String,
    pub request_id: String,
    pub mime_type: String,
    #[serde(default)]
    pub file_name: Option<String>,
    pub bytes_base64: String,
    pub size_bytes: u64,
    pub sha256_hex: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub uses_remaining: u32,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct FinishRequestRequest {
    pub request_id: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct FinishRequestResponse {
    pub request_id: String,
    pub removed_tokens: usize,
    pub removed_bytes: u64,
}

async fn call_blobstore_tool<T, P>(
    plugin_manager: &PluginManager,
    tool_name: &str,
    request: &P,
) -> Result<T>
where
    T: serde::de::DeserializeOwned,
    P: Serialize,
{
    let arguments_json = serde_json::to_string(request)?;
    let result = plugin_manager
        .call_tool_by_capability(OBJECT_STORE_CAPABILITY, tool_name, &arguments_json)
        .await?;
    if result.is_error {
        bail!("{}", result.content_json);
    }
    serde_json::from_str(&result.content_json)
        .map_err(|err| anyhow::anyhow!("Decode blobstore tool result for '{tool_name}': {err}"))
}

pub async fn object_store_available(plugin_manager: &PluginManager) -> bool {
    plugin_manager
        .is_capability_available(OBJECT_STORE_CAPABILITY)
        .await
}

#[allow(dead_code)]
pub async fn put_request_object(
    plugin_manager: &PluginManager,
    request: PutRequestObjectRequest,
) -> Result<PutRequestObjectResponse> {
    call_blobstore_tool(plugin_manager, PUT_REQUEST_OBJECT_TOOL, &request).await
}

#[allow(dead_code)]
pub async fn get_request_object(
    plugin_manager: &PluginManager,
    request: GetRequestObjectRequest,
) -> Result<GetRequestObjectResponse> {
    call_blobstore_tool(plugin_manager, GET_REQUEST_OBJECT_TOOL, &request).await
}

#[allow(dead_code)]
pub async fn complete_request(
    plugin_manager: &PluginManager,
    request: FinishRequestRequest,
) -> Result<FinishRequestResponse> {
    call_blobstore_tool(plugin_manager, COMPLETE_REQUEST_TOOL, &request).await
}

#[allow(dead_code)]
pub async fn abort_request(
    plugin_manager: &PluginManager,
    request: FinishRequestRequest,
) -> Result<FinishRequestResponse> {
    call_blobstore_tool(plugin_manager, ABORT_REQUEST_TOOL, &request).await
}
