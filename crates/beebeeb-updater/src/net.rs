//! The https client (feature `net`). It speaks https only, follows no redirect, and sends nothing about the person:
//! a fixed user agent and, when resuming, a `Range` header.
//!
//! Proxies: reqwest's defaults, as the engine's own client uses: `HTTPS_PROXY`/`ALL_PROXY`/`NO_PROXY`, and on
//! Windows the system proxy from Internet Settings. TLS is the OS stack (Schannel on Windows), so a company's own
//! root certificate is honoured; what is installed is decided by the signatures, not by TLS.

use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::{CONTENT_RANGE, RANGE, USER_AGENT};

use crate::machine::{Http, Response, Unreachable};

/// One request, body included, may take this long. A slower download is cut and resumed from where it stopped.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

pub struct HttpsClient {
    client: Client,
}

impl HttpsClient {
    pub fn new() -> Result<Self, Unreachable> {
        Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map(|client| Self { client })
            .map_err(|_| Unreachable)
    }
}

impl Http for HttpsClient {
    fn get(&mut self, url: &str, from: u64) -> Result<Response, Unreachable> {
        let mut request = self
            .client
            .get(url)
            .header(USER_AGENT, concat!("beebeeb-updater/", env!("CARGO_PKG_VERSION")));
        if from > 0 {
            request = request.header(RANGE, format!("bytes={from}-"));
        }
        let response = request.send().map_err(|_| Unreachable)?;
        let status = response.status().as_u16();
        let (offset, total) = if status == 206 {
            // `Content-Range: bytes <first>-<last>/<total>`. A 206 without a readable one is reported at an offset
            // no request asked for, so the machine refuses it.
            response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(parse_content_range)
                .unwrap_or((u64::MAX, None))
        } else {
            (0, response.content_length())
        };
        Ok(Response {
            status,
            offset,
            total,
            body: Box::new(response),
        })
    }
}

fn parse_content_range(value: &str) -> Option<(u64, Option<u64>)> {
    let (range, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let first = range.split_once('-')?.0.parse().ok()?;
    Some((first, total.parse().ok()))
}

#[cfg(test)]
mod tests {
    use super::parse_content_range;

    #[test]
    fn reads_the_first_byte_and_the_total_of_a_content_range() {
        assert_eq!(
            parse_content_range("bytes 4096-13134242/13134243"),
            Some((4096, Some(13134243)))
        );
        assert_eq!(parse_content_range("bytes 4096-8191/*"), Some((4096, None)));
        assert_eq!(parse_content_range("items 0-1/2"), None);
        assert_eq!(parse_content_range("bytes */100"), None);
    }
}
