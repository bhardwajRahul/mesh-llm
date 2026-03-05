//! HTTP proxy plumbing — request parsing, model routing, response helpers.
//!
//! Used by the API proxy (port 9337), bootstrap proxy, and passive mode.
//! All inference traffic flows through these functions.

use crate::{election, mesh, tunnel};
use anyhow::Result;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

// ── Request parsing ──

/// Peek at an HTTP request without consuming it. Returns bytes peeked and optional model name.
pub async fn peek_request(stream: &TcpStream, buf: &mut [u8]) -> Result<(usize, Option<String>)> {
    let n = stream.peek(buf).await?;
    if n == 0 {
        anyhow::bail!("Empty request");
    }
    let model = extract_model_from_http(&buf[..n]);
    Ok((n, model))
}

/// Extract `"model"` field from a JSON POST body in an HTTP request.
pub fn extract_model_from_http(buf: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(buf).ok()?;
    let body_start = s.find("\r\n\r\n")? + 4;
    let body = &s[body_start..];
    let model_key = "\"model\"";
    let pos = body.find(model_key)?;
    let after_key = &body[pos + model_key.len()..];
    let after_colon = after_key.trim_start().strip_prefix(':')?;
    let after_ws = after_colon.trim_start();
    let after_quote = after_ws.strip_prefix('"')?;
    let end = after_quote.find('"')?;
    Some(after_quote[..end].to_string())
}

/// Extract a session hint from an HTTP request for MoE sticky routing.
/// Looks for "user" or "session_id" in the JSON body. Falls back to None.
pub fn extract_session_hint(buf: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(buf).ok()?;
    let body_start = s.find("\r\n\r\n")? + 4;
    let body = &s[body_start..];
    // Try "user" field first (standard OpenAI parameter)
    for key in &["\"user\"", "\"session_id\""] {
        if let Some(pos) = body.find(key) {
            let after_key = &body[pos + key.len()..];
            let after_colon = after_key.trim_start().strip_prefix(':')?;
            let after_ws = after_colon.trim_start();
            let after_quote = after_ws.strip_prefix('"')?;
            let end = after_quote.find('"')?;
            return Some(after_quote[..end].to_string());
        }
    }
    None
}

pub fn is_models_list_request(buf: &[u8]) -> bool {
    let s = String::from_utf8_lossy(buf);
    s.starts_with("GET ") && (s.contains("/v1/models") || s.contains("/models"))
        && !s.contains("/v1/models/")
}

pub fn is_drop_request(buf: &[u8]) -> bool {
    let s = String::from_utf8_lossy(buf);
    s.starts_with("POST ") && s.contains("/mesh/drop")
}

// ── Model-aware tunnel routing ──

/// The common request-handling path used by idle proxy, passive proxy, and bootstrap proxy.
///
/// Peeks at the HTTP request, handles `/v1/models`, resolves the target host
/// by model name (or falls back to any host), and tunnels the request via QUIC.
///
/// Set `track_demand` to record requests for demand-based rebalancing.
pub async fn handle_mesh_request(node: mesh::Node, tcp_stream: TcpStream, track_demand: bool) {
    let mut buf = vec![0u8; 32768];
    let (n, model_name) = match peek_request(&tcp_stream, &mut buf).await {
        Ok(v) => v,
        Err(_) => return,
    };

    // Handle /v1/models
    if is_models_list_request(&buf[..n]) {
        let served = node.models_being_served().await;
        let _ = send_models_list(tcp_stream, &served).await;
        return;
    }

    // Demand tracking for rebalancing
    if track_demand {
        if let Some(ref name) = model_name {
            node.record_request(name);
        }
    }

    // Resolve target hosts by model name, fall back to any host
    let target_hosts = if let Some(ref name) = model_name {
        node.hosts_for_model(name).await
    } else {
        vec![]
    };
    let target_hosts = if target_hosts.is_empty() {
        match node.any_host().await {
            Some(p) => vec![p.id],
            None => {
                let _ = send_503(tcp_stream).await;
                return;
            }
        }
    } else {
        target_hosts
    };

    // Try each host in order — if tunnel fails, retry with next.
    // On first failure, trigger background gossip refresh so future requests
    // have a fresh routing table (doesn't block the retry loop).
    let mut last_err = None;
    let mut refreshed = false;
    for target_host in &target_hosts {
        match node.open_http_tunnel(*target_host).await {
            Ok((quic_send, quic_recv)) => {
                if let Err(e) = tunnel::relay_tcp_via_quic(tcp_stream, quic_send, quic_recv).await {
                    tracing::debug!("HTTP tunnel relay ended: {e}");
                }
                return;
            }
            Err(e) => {
                tracing::warn!("Failed to tunnel to host {}: {e}, trying next", target_host.fmt_short());
                last_err = Some(e);
                // Background refresh on first failure — non-blocking
                if !refreshed {
                    let refresh_node = node.clone();
                    tokio::spawn(async move { refresh_node.gossip_one_peer().await; });
                    refreshed = true;
                }
            }
        }
    }
    // All hosts failed
    if let Some(e) = last_err {
        tracing::warn!("All hosts failed for model {:?}: {e}", model_name);
    }
    let _ = send_503(tcp_stream).await;
}

/// Route a request to a known inference target (local llama-server or remote host).
///
/// Used by the API proxy after election has determined the target.
pub async fn route_to_target(node: mesh::Node, tcp_stream: TcpStream, target: election::InferenceTarget) {
    match target {
        election::InferenceTarget::Local(port) | election::InferenceTarget::MoeLocal(port) => {
            match TcpStream::connect(format!("127.0.0.1:{port}")).await {
                Ok(upstream) => {
                    let _inflight = node.begin_inflight_request();
                    let _ = upstream.set_nodelay(true);
                    if let Err(e) = tunnel::relay_tcp_streams(tcp_stream, upstream).await {
                        tracing::debug!("API proxy (local) ended: {e}");
                    }
                }
                Err(e) => {
                    tracing::warn!("API proxy: can't reach llama-server on {port}: {e}");
                    let _ = send_503(tcp_stream).await;
                }
            }
        }
        election::InferenceTarget::Remote(host_id) | election::InferenceTarget::MoeRemote(host_id) => {
            match node.open_http_tunnel(host_id).await {
                Ok((quic_send, quic_recv)) => {
                    if let Err(e) = tunnel::relay_tcp_via_quic(tcp_stream, quic_send, quic_recv).await {
                        tracing::debug!("API proxy (remote) ended: {e}");
                    }
                }
                Err(e) => {
                    tracing::warn!("API proxy: can't tunnel to host {}: {e}", host_id.fmt_short());
                    let _ = send_503(tcp_stream).await;
                }
            }
        }
        election::InferenceTarget::None => {
            let _ = send_503(tcp_stream).await;
        }
    }
}

// ── Response helpers ──

pub async fn send_models_list(mut stream: TcpStream, models: &[String]) -> std::io::Result<()> {
    let data: Vec<serde_json::Value> = models
        .iter()
        .map(|m| {
            serde_json::json!({
                "id": m,
                "object": "model",
                "owned_by": "mesh-llm",
            })
        })
        .collect();
    let body = serde_json::json!({ "object": "list", "data": data }).to_string();
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\n{}",
        body.len(), body
    );
    stream.write_all(resp.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

pub async fn send_json_ok(mut stream: TcpStream, data: &serde_json::Value) -> std::io::Result<()> {
    let body = data.to_string();
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(), body
    );
    stream.write_all(resp.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

pub async fn send_400(mut stream: TcpStream, msg: &str) -> std::io::Result<()> {
    let body = format!("{{\"error\":\"{msg}\"}}");
    let resp = format!(
        "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(), body
    );
    stream.write_all(resp.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

pub async fn send_503(mut stream: TcpStream) -> std::io::Result<()> {
    let body = r#"{"error":"No inference server available — election in progress"}"#;
    let resp = format!(
        "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(), body
    );
    stream.write_all(resp.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

// ── MoE probe-based session placement ──

/// Probe all MoE shard nodes in parallel to find the best one for a prompt.
/// Each node generates 1 token with logprobs; the node with the highest
/// mean logprob is the best match for this prompt's expert routing.
///
/// Returns the index into `moe_nodes` of the winning node, or None on failure.
pub async fn probe_moe_nodes(
    node: &mesh::Node,
    moe_nodes: &[election::InferenceTarget],
    prompt: &str,
    model_name: &str,
) -> Option<usize> {
    if moe_nodes.len() <= 1 {
        return Some(0);
    }

    let mut handles = Vec::with_capacity(moe_nodes.len());

    for (idx, target) in moe_nodes.iter().enumerate() {
        let prompt = prompt.to_string();
        let model = model_name.to_string();
        let target = target.clone();
        let node = node.clone();

        let handle = tokio::spawn(async move {
            let result = match target {
                election::InferenceTarget::MoeLocal(port)
                | election::InferenceTarget::Local(port) => {
                    probe_local(port, &prompt, &model).await
                }
                election::InferenceTarget::MoeRemote(peer_id)
                | election::InferenceTarget::Remote(peer_id) => {
                    probe_remote(&node, peer_id, &prompt, &model).await
                }
                _ => Err(anyhow::anyhow!("Can't probe InferenceTarget::None")),
            };
            (idx, result)
        });
        handles.push(handle);
    }

    // Collect results with a timeout
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut scores: Vec<(usize, f64)> = Vec::new();

    for handle in handles {
        match tokio::time::timeout_at(deadline, handle).await {
            Ok(Ok((idx, Ok(logprob)))) => {
                scores.push((idx, logprob));
            }
            Ok(Ok((idx, Err(e)))) => {
                tracing::debug!("MoE probe failed for node {idx}: {e}");
            }
            _ => {
                tracing::debug!("MoE probe timed out or cancelled");
            }
        }
    }

    if scores.is_empty() {
        return None;
    }

    // Pick the node with the highest (least negative) mean logprob
    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let winner = scores[0].0;

    if scores.len() > 1 {
        eprintln!("🎯 MoE probe: node {} wins (logprob {:.3} vs {:.3})",
            winner,
            scores[0].1,
            scores[1].1);
    }

    Some(winner)
}

/// Probe a local llama-server shard for logprob score.
async fn probe_local(port: u16, prompt: &str, model: &str) -> Result<f64> {
    let url = format!("http://127.0.0.1:{port}/v1/chat/completions");
    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": 1,
        "temperature": 0.0,
        "logprobs": true,
        "top_logprobs": 1,
    });

    let resp = reqwest::Client::new()
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(4))
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;

    extract_logprob(&resp)
}

/// Probe a remote MoE shard via QUIC tunnel.
async fn probe_remote(
    node: &mesh::Node,
    peer_id: iroh::EndpointId,
    prompt: &str,
    model: &str,
) -> Result<f64> {
    let (mut send, mut recv) = node.open_http_tunnel(peer_id).await?;

    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": 1,
        "temperature": 0.0,
        "logprobs": true,
        "top_logprobs": 1,
    });
    let body_bytes = serde_json::to_vec(&body)?;

    let request = format!(
        "POST /v1/chat/completions HTTP/1.1\r\n\
         Host: localhost\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n",
        body_bytes.len()
    );

    send.write_all(request.as_bytes()).await?;
    send.write_all(&body_bytes).await?;
    send.finish()?;

    // Read the full response
    let response = recv.read_to_end(1024 * 1024).await?;
    let response_str = String::from_utf8_lossy(&response);

    // Find the JSON body after headers
    let json_start = response_str.find("\r\n\r\n")
        .or_else(|| response_str.find("\n\n"))
        .map(|i| if response_str[i..].starts_with("\r\n\r\n") { i + 4 } else { i + 2 })
        .ok_or_else(|| anyhow::anyhow!("No HTTP body in probe response"))?;

    let resp: serde_json::Value = serde_json::from_str(&response_str[json_start..])?;
    extract_logprob(&resp)
}

/// Extract mean logprob from an OpenAI chat completion response.
fn extract_logprob(resp: &serde_json::Value) -> Result<f64> {
    // Path: choices[0].logprobs.content[0].logprob
    let content = resp
        .get("choices").and_then(|c| c.get(0))
        .and_then(|c| c.get("logprobs"))
        .and_then(|lp| lp.get("content"))
        .and_then(|c| c.as_array())
        .ok_or_else(|| anyhow::anyhow!("No logprobs in response: {}", resp))?;

    if content.is_empty() {
        anyhow::bail!("Empty logprobs content array");
    }

    let sum: f64 = content.iter()
        .filter_map(|entry| entry.get("logprob").and_then(|v| v.as_f64()))
        .sum();
    let count = content.iter()
        .filter(|entry| entry.get("logprob").and_then(|v| v.as_f64()).is_some())
        .count();

    if count == 0 {
        anyhow::bail!("No valid logprob values");
    }

    Ok(sum / count as f64)
}

/// Extract prompt text from an HTTP request buffer for probing.
/// Pulls the last user message content from the chat completions body.
pub fn extract_prompt_for_probe(buf: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(buf).ok()?;
    let body_start = s.find("\r\n\r\n").map(|i| i + 4)
        .or_else(|| s.find("\n\n").map(|i| i + 2))?;
    let body = &s[body_start..];
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    let messages = json.get("messages")?.as_array()?;
    // Get last user message
    messages.iter().rev()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_session_hint_user_field() {
        let req = b"POST /v1/chat/completions HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{\"model\":\"qwen\",\"user\":\"alice\",\"messages\":[]}";
        assert_eq!(extract_session_hint(req), Some("alice".to_string()));
    }

    #[test]
    fn test_extract_session_hint_session_id() {
        let req = b"POST /v1/chat/completions HTTP/1.1\r\n\r\n{\"model\":\"qwen\",\"session_id\":\"sess-42\"}";
        assert_eq!(extract_session_hint(req), Some("sess-42".to_string()));
    }

    #[test]
    fn test_extract_session_hint_user_preferred_over_session_id() {
        // "user" appears before "session_id" in our search order
        let req = b"POST /v1/chat/completions HTTP/1.1\r\n\r\n{\"user\":\"bob\",\"session_id\":\"sess-1\"}";
        assert_eq!(extract_session_hint(req), Some("bob".to_string()));
    }

    #[test]
    fn test_extract_session_hint_none() {
        let req = b"POST /v1/chat/completions HTTP/1.1\r\n\r\n{\"model\":\"qwen\",\"messages\":[]}";
        assert_eq!(extract_session_hint(req), None);
    }

    #[test]
    fn test_extract_session_hint_no_body() {
        let req = b"GET /v1/models HTTP/1.1\r\n\r\n";
        assert_eq!(extract_session_hint(req), None);
    }

    #[test]
    fn test_extract_session_hint_no_headers_end() {
        let req = b"POST /v1/chat body without proper headers";
        assert_eq!(extract_session_hint(req), None);
    }

    #[test]
    fn test_extract_session_hint_whitespace_variants() {
        // Extra whitespace around colon and value
        let req = b"POST / HTTP/1.1\r\n\r\n{\"user\" : \"charlie\" }";
        assert_eq!(extract_session_hint(req), Some("charlie".to_string()));
    }

    #[test]
    fn test_extract_session_hint_empty_value() {
        let req = b"POST / HTTP/1.1\r\n\r\n{\"user\":\"\"}";
        assert_eq!(extract_session_hint(req), Some("".to_string()));
    }

    #[test]
    fn test_extract_model_from_http_basic() {
        let req = b"POST /v1/chat/completions HTTP/1.1\r\n\r\n{\"model\":\"Qwen3-30B\"}";
        assert_eq!(extract_model_from_http(req), Some("Qwen3-30B".to_string()));
    }

    #[test]
    fn test_extract_logprob_real_response() {
        // Real response from llama-server with logprobs:true, max_tokens:1
        let resp: serde_json::Value = serde_json::from_str(r#"{
            "choices":[{
                "finish_reason":"length",
                "index":0,
                "message":{"role":"assistant","content":"1"},
                "logprobs":{"content":[{
                    "id":16,"token":"1","bytes":[49],
                    "logprob":-0.501800537109375,
                    "top_logprobs":[{"id":16,"token":"1","bytes":[49],"logprob":-0.501800537109375}]
                }]}
            }],
            "model":"GLM-4.7-Flash-Q4_K_M.gguf",
            "object":"chat.completion"
        }"#).unwrap();
        let lp = extract_logprob(&resp).unwrap();
        assert!((lp - (-0.501800537109375)).abs() < 1e-6);
    }

    #[test]
    fn test_extract_logprob_no_logprobs() {
        let resp: serde_json::Value = serde_json::from_str(r#"{
            "choices":[{"message":{"content":"hi"}}]
        }"#).unwrap();
        assert!(extract_logprob(&resp).is_err());
    }

    #[test]
    fn test_extract_logprob_empty_content() {
        let resp: serde_json::Value = serde_json::from_str(r#"{
            "choices":[{"logprobs":{"content":[]}}]
        }"#).unwrap();
        assert!(extract_logprob(&resp).is_err());
    }

    #[test]
    fn test_extract_prompt_for_probe() {
        let req = b"POST /v1/chat/completions HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{\"model\":\"test\",\"messages\":[{\"role\":\"system\",\"content\":\"You are helpful\"},{\"role\":\"user\",\"content\":\"What is 2+2?\"}]}";
        assert_eq!(extract_prompt_for_probe(req), Some("What is 2+2?".to_string()));
    }

    #[test]
    fn test_extract_prompt_for_probe_no_user() {
        let req = b"POST / HTTP/1.1\r\n\r\n{\"messages\":[{\"role\":\"system\",\"content\":\"hi\"}]}";
        assert_eq!(extract_prompt_for_probe(req), None);
    }
}
