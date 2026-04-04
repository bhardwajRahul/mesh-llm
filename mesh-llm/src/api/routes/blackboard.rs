use super::super::{http::respond_error, MeshApi};
use crate::plugin;
use serde_json::json;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

pub(super) async fn handle(
    stream: &mut TcpStream,
    state: &MeshApi,
    method: &str,
    path: &str,
    body: &str,
) -> anyhow::Result<()> {
    match (method, path.split('?').next().unwrap_or(path)) {
        ("GET", "/api/blackboard/feed") => handle_feed(stream, state, path).await,
        ("GET", "/api/blackboard/search") => handle_search(stream, state, path).await,
        ("POST", "/api/blackboard/post") => handle_post(stream, state, body).await,
        _ => Ok(()),
    }
}

async fn handle_feed(stream: &mut TcpStream, state: &MeshApi, path: &str) -> anyhow::Result<()> {
    let plugin_manager = state.inner.lock().await.plugin_manager.clone();
    if !plugin_manager
        .is_capability_available(plugin::BLACKBOARD_CAPABILITY)
        .await
    {
        respond_error(stream, 404, "Blackboard is disabled on this node").await?;
        return Ok(());
    }

    let params = query_params(path);
    let args = json!({
        "from": params.iter().find(|(k, _)| *k == "from").map(|(_, v)| v.to_string()),
        "limit": params.iter().find(|(k, _)| *k == "limit").and_then(|(_, v)| v.parse::<usize>().ok()).unwrap_or(20),
        "since": params.iter().find(|(k, _)| *k == "since").and_then(|(_, v)| v.parse::<u64>().ok()).unwrap_or(0),
    })
    .to_string();
    match plugin_manager
        .call_tool_by_capability(plugin::BLACKBOARD_CAPABILITY, "feed", &args)
        .await
    {
        Ok(result) if !result.is_error => {
            let json = &result.content_json;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                json.len(),
                json
            );
            stream.write_all(resp.as_bytes()).await?;
        }
        Ok(result) => {
            respond_error(stream, 502, &result.content_json).await?;
        }
        Err(e) => {
            respond_error(stream, 502, &e.to_string()).await?;
        }
    }
    Ok(())
}

async fn handle_search(stream: &mut TcpStream, state: &MeshApi, path: &str) -> anyhow::Result<()> {
    let plugin_manager = state.inner.lock().await.plugin_manager.clone();
    if !plugin_manager
        .is_capability_available(plugin::BLACKBOARD_CAPABILITY)
        .await
    {
        respond_error(stream, 404, "Blackboard is disabled on this node").await?;
        return Ok(());
    }

    let params = query_params(path);
    let args = json!({
        "query": params.iter().find(|(k, _)| *k == "q").map(|(_, v)| v.replace('+', " ").replace("%20", " ")).unwrap_or_default(),
        "limit": params.iter().find(|(k, _)| *k == "limit").and_then(|(_, v)| v.parse::<usize>().ok()).unwrap_or(20),
        "since": params.iter().find(|(k, _)| *k == "since").and_then(|(_, v)| v.parse::<u64>().ok()).unwrap_or(0),
    })
    .to_string();
    match plugin_manager
        .call_tool_by_capability(plugin::BLACKBOARD_CAPABILITY, "search", &args)
        .await
    {
        Ok(result) if !result.is_error => {
            let json = &result.content_json;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                json.len(),
                json
            );
            stream.write_all(resp.as_bytes()).await?;
        }
        Ok(result) => {
            respond_error(stream, 502, &result.content_json).await?;
        }
        Err(e) => {
            respond_error(stream, 502, &e.to_string()).await?;
        }
    }
    Ok(())
}

async fn handle_post(stream: &mut TcpStream, state: &MeshApi, body: &str) -> anyhow::Result<()> {
    let (node, plugin_manager) = {
        let inner = state.inner.lock().await;
        (inner.node.clone(), inner.plugin_manager.clone())
    };
    if !plugin_manager
        .is_capability_available(plugin::BLACKBOARD_CAPABILITY)
        .await
    {
        respond_error(stream, 404, "Blackboard is disabled on this node").await?;
        return Ok(());
    }

    let parsed: Result<serde_json::Value, _> = serde_json::from_str(body);
    match parsed {
        Ok(val) => {
            let text = val["text"].as_str().unwrap_or("").to_string();
            if text.is_empty() {
                respond_error(stream, 400, "Missing 'text' field").await?;
            } else {
                let args = json!({
                    "text": text,
                    "from": node.display_name().await,
                    "peer_id": node.id().fmt_short().to_string(),
                })
                .to_string();
                match plugin_manager
                    .call_tool_by_capability(plugin::BLACKBOARD_CAPABILITY, "post", &args)
                    .await
                {
                    Ok(result) if !result.is_error => {
                        let json = result.content_json;
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                            json.len(),
                            json
                        );
                        stream.write_all(resp.as_bytes()).await?;
                    }
                    Ok(result) => {
                        let status = if result.content_json.contains("Rate limited") {
                            429
                        } else {
                            400
                        };
                        respond_error(stream, status, &result.content_json).await?;
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        let status = if msg.contains("Rate limited") {
                            429
                        } else {
                            400
                        };
                        respond_error(stream, status, &msg).await?;
                    }
                }
            }
        }
        Err(_) => {
            respond_error(stream, 400, "Invalid JSON body").await?;
        }
    }
    Ok(())
}

fn query_params(path: &str) -> Vec<(&str, &str)> {
    path.split('?')
        .nth(1)
        .unwrap_or("")
        .split('&')
        .filter_map(|part| part.split_once('='))
        .collect()
}
