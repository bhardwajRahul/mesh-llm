use crate::network::openai::transport;
use anyhow::Result;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

#[derive(Debug)]
pub(crate) struct BackendProxyHandle {
    port: u16,
    task: tokio::task::JoinHandle<()>,
}

impl BackendProxyHandle {
    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    pub(crate) async fn shutdown(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}

pub(crate) async fn start_backend_proxy(llama_port: u16) -> Result<BackendProxyHandle> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();

    let task = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    tokio::spawn(async move {
                        if let Err(err) = handle_connection(stream, llama_port).await {
                            tracing::debug!("backend proxy request failed: {err}");
                        }
                    });
                }
                Err(err) => {
                    tracing::warn!("backend proxy accept error: {err}");
                }
            }
        }
    });

    Ok(BackendProxyHandle { port, task })
}

async fn handle_connection(mut stream: TcpStream, llama_port: u16) -> Result<()> {
    let _ = stream.set_nodelay(true);
    let request = match transport::read_http_request(&mut stream).await {
        Ok(request) => request,
        Err(err) => {
            let _ = transport::send_400(stream, &err.to_string()).await;
            return Ok(());
        }
    };

    let mut upstream = match TcpStream::connect(format!("127.0.0.1:{llama_port}")).await {
        Ok(stream) => stream,
        Err(err) => {
            let _ = transport::send_503(stream, &format!("llama backend unavailable: {err}")).await;
            return Ok(());
        }
    };
    let _ = upstream.set_nodelay(true);

    if let Err(err) = upstream.write_all(&request.raw).await {
        let _ = transport::send_503(stream, &format!("llama backend write failed: {err}")).await;
        return Ok(());
    }

    // The buffered request is already written to upstream; relay the response back to the client.
    let _ = tokio::io::copy(&mut upstream, &mut stream).await;
    let _ = stream.shutdown().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Starts a minimal dummy HTTP server that echoes a fixed response for every connection.
    async fn start_dummy_upstream(response: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut conn, _)) = listener.accept().await {
                let response = response;
                tokio::spawn(async move {
                    // Drain the incoming request so the client gets its response.
                    let mut buf = [0u8; 4096];
                    let _ = conn.read(&mut buf).await;
                    let _ = conn.write_all(response.as_bytes()).await;
                });
            }
        });
        port
    }

    #[tokio::test]
    async fn test_backend_proxy_forwards_request_and_response() {
        let upstream_response =
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}";
        let upstream_port = start_dummy_upstream(upstream_response).await;

        let proxy = start_backend_proxy(upstream_port).await.unwrap();
        let proxy_port = proxy.port();

        let mut client = TcpStream::connect(format!("127.0.0.1:{proxy_port}"))
            .await
            .unwrap();
        let req = b"POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}";
        client.write_all(req).await.unwrap();

        let mut buf = Vec::new();
        client.read_to_end(&mut buf).await.unwrap();
        let response = String::from_utf8_lossy(&buf);
        assert!(
            response.starts_with("HTTP/1.1 200 OK"),
            "unexpected response: {response:?}"
        );
        assert!(response.contains("{}"), "body not forwarded: {response:?}");

        proxy.shutdown().await;
    }

    #[tokio::test]
    async fn test_backend_proxy_shutdown_stops_accepting() {
        let upstream_response =
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}";
        let upstream_port = start_dummy_upstream(upstream_response).await;

        let proxy = start_backend_proxy(upstream_port).await.unwrap();
        let proxy_port = proxy.port();

        // Confirm proxy accepts connections before shutdown.
        TcpStream::connect(format!("127.0.0.1:{proxy_port}"))
            .await
            .expect("should connect before shutdown");

        proxy.shutdown().await;

        // After shutdown the accept loop is aborted; new connections should be refused.
        // Give the OS a moment to release the port then verify connection is refused.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let result = TcpStream::connect(format!("127.0.0.1:{proxy_port}")).await;
        assert!(
            result.is_err(),
            "connection should be refused after shutdown"
        );
    }

    #[tokio::test]
    async fn test_backend_proxy_returns_503_when_upstream_unavailable() {
        // Use a port that has nothing listening on it.
        let dead_port = {
            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
            l.local_addr().unwrap().port()
            // listener dropped — port freed
        };

        let proxy = start_backend_proxy(dead_port).await.unwrap();
        let proxy_port = proxy.port();

        let mut client = TcpStream::connect(format!("127.0.0.1:{proxy_port}"))
            .await
            .unwrap();
        let req = b"POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}";
        client.write_all(req).await.unwrap();

        let mut buf = Vec::new();
        client.read_to_end(&mut buf).await.unwrap();
        let response = String::from_utf8_lossy(&buf);
        assert!(
            response.starts_with("HTTP/1.1 503"),
            "expected 503, got: {response:?}"
        );

        proxy.shutdown().await;
    }
}
