//! Single-attempt requests for the unknown-file check. The caller reserves each request.
use super::ApiClient;
use crate::unknown_check::compare::HttpFailure;

impl ApiClient {
    pub(crate) async fn unknown_get(&self, url: &str, max: u64) -> anyhow::Result<Vec<u8>> {
        let mut response = self
            .client
            .get(url)
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await
            .map_err(|_| crate::unknown_check::compare::Halt::Offline)?;
        if !response.status().is_success() {
            let retry = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| {
                    v.parse::<u64>().ok().or_else(|| {
                        chrono::DateTime::parse_from_rfc2822(v)
                            .ok()
                            .map(|date| (date.timestamp() - chrono::Utc::now().timestamp()).max(0) as u64)
                    })
                })
                .unwrap_or(0);
            return Err(HttpFailure(response.status().as_u16(), retry).into());
        }
        let mut bytes = Vec::new();
        while let Some(piece) = response
            .chunk()
            .await
            .map_err(|_| crate::unknown_check::compare::Halt::Offline)?
        {
            if (bytes.len() as u64).saturating_add(piece.len() as u64) > max {
                anyhow::bail!("check response exceeds its reservation");
            }
            bytes.extend_from_slice(&piece);
        }
        Ok(bytes)
    }
    pub(crate) async fn unknown_json(&self, url: &str) -> anyhow::Result<serde_json::Value> {
        Ok(serde_json::from_slice(&self.unknown_get(url, 16 * 1024 * 1024).await?)?)
    }
    pub(crate) async fn unknown_chunk(
        &self,
        file: &str,
        index: u32,
        pin: Option<&crate::download_pin::PinnedVersion>,
        max: u64,
    ) -> anyhow::Result<Vec<u8>> {
        let mut url = self.chunk_url(file, index);
        if let Some(pin) = pin {
            url.push_str(&format!(
                "?object_version_id={}",
                urlencoding::encode(&pin.object_version_id)
            ));
        }
        let bytes = self.unknown_get(&url, max).await?;
        if let Some(pin) = pin
            && pin.wire_size(index) != Some(bytes.len() as u64)
        {
            anyhow::bail!("check chunk differs from manifest size");
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn task_2191_s2_http_429_keeps_retry_after_and_never_retries() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop2 = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut count = 0;
            let started = std::time::Instant::now();
            while !stop2.load(std::sync::atomic::Ordering::SeqCst) && started.elapsed().as_secs() < 5 {
                if let Ok((mut socket, _)) = listener.accept() {
                    socket
                        .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                        .unwrap();
                    let mut b = [0; 4096];
                    let _ = socket.read(&mut b);
                    socket.write_all(b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 901\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    count += 1;
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
            count
        });
        let api = ApiClient::new(url.clone(), "test".into(), [0; 32]);
        let error = api.unknown_get(&url, 1024).await.unwrap_err();
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        let count = thread.join().unwrap();
        assert_eq!(count, 1);
        let http = error.downcast_ref::<HttpFailure>().expect("HTTP status retained");
        assert_eq!((http.0, http.1), (429, 901));
    }
}
