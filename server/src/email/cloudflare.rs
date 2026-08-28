//! Production Cloudflare Email Service implementation.
//!
//! Sends via the Email Sending REST API:
//! `POST https://api.cloudflare.com/client/v4/accounts/{account_id}/email/sending/send`
//! authenticated with an API token holding the "Email Sending: Edit"
//! permission. The sender domain must be onboarded for Email Sending on the
//! account that owns the token.

use super::{EmailError, EmailService};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Production [`EmailService`] backed by Cloudflare Email Service.
///
/// The [`reqwest::Client`] is constructed once at startup and reused for
/// connection pooling, mirroring the SES client lifecycle.
pub struct CloudflareEmailService {
    http: reqwest::Client,
    send_url: String,
    api_token: String,
    from_address: String,
}

#[derive(Serialize)]
struct SendRequest<'a> {
    to: &'a str,
    from: &'a str,
    subject: &'a str,
    html: &'a str,
    text: &'a str,
}

/// Cloudflare v4 API envelope. External response — no `deny_unknown_fields`
/// so added fields never break parsing.
#[derive(Deserialize)]
struct CfEnvelope {
    success: bool,
    #[serde(default)]
    errors: Vec<CfApiError>,
}

#[derive(Deserialize)]
struct CfApiError {
    #[serde(default)]
    code: Option<i64>,
    #[serde(default)]
    message: String,
}

impl CloudflareEmailService {
    pub fn new(account_id: &str, api_token: String, from_address: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            send_url: format!(
                "https://api.cloudflare.com/client/v4/accounts/{account_id}/email/sending/send"
            ),
            api_token,
            from_address,
        }
    }

    /// Test constructor pointing at an arbitrary endpoint.
    #[cfg(test)]
    fn with_send_url(send_url: String, api_token: String, from_address: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            send_url,
            api_token,
            from_address,
        }
    }
}

#[async_trait]
impl EmailService for CloudflareEmailService {
    async fn send_email(
        &self,
        to: &str,
        subject: &str,
        body_html: &str,
        body_text: &str,
    ) -> Result<(), EmailError> {
        let resp = self
            .http
            .post(&self.send_url)
            .bearer_auth(&self.api_token)
            .timeout(std::time::Duration::from_secs(15))
            .json(&SendRequest {
                to,
                from: &self.from_address,
                subject,
                html: body_html,
                text: body_text,
            })
            .send()
            .await
            .map_err(|e| EmailError::SendFailure(format!("cloudflare request failed: {e}")))?;

        let status = resp.status();
        let envelope: CfEnvelope = resp
            .json()
            .await
            .map_err(|e| EmailError::SendFailure(format!("cloudflare response ({status}): {e}")))?;

        if !envelope.success {
            let detail = envelope
                .errors
                .iter()
                .map(|e| match e.code {
                    Some(code) => format!("{code}: {}", e.message),
                    None => e.message.clone(),
                })
                .collect::<Vec<_>>()
                .join("; ");
            return Err(EmailError::SendFailure(format!(
                "cloudflare send rejected ({status}): {detail}"
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// One-shot HTTP server that captures the request and returns `response`.
    async fn capture_one_request(
        response: &'static str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 64 * 1024];
            let n = stream.read(&mut buf).await.unwrap();
            let captured = String::from_utf8_lossy(&buf[..n]).to_string();
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.shutdown().await.ok();
            captured
        });
        (format!("http://{addr}/send"), handle)
    }

    #[tokio::test]
    async fn sends_bearer_token_and_json_payload() {
        let body = r#"{"success":true,"errors":[],"messages":[],"result":{}}"#;
        let (url, handle) = capture_one_request(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 54\r\n\r\n{\"success\":true,\"errors\":[],\"messages\":[],\"result\":{}}",
        )
        .await;
        assert_eq!(body.len(), 54);

        let svc = CloudflareEmailService::with_send_url(
            url,
            "tok-123".into(),
            "noreply@ololo.dev".into(),
        );
        svc.send_email("user@example.com", "Hi", "<b>hi</b>", "hi")
            .await
            .expect("send ok");

        let captured = handle.await.unwrap();
        assert!(captured.contains("authorization: Bearer tok-123"));
        let json_start = captured.find("\r\n\r\n").unwrap() + 4;
        let payload: serde_json::Value = serde_json::from_str(&captured[json_start..]).unwrap();
        assert_eq!(payload["to"], "user@example.com");
        assert_eq!(payload["from"], "noreply@ololo.dev");
        assert_eq!(payload["subject"], "Hi");
        assert_eq!(payload["html"], "<b>hi</b>");
        assert_eq!(payload["text"], "hi");
    }

    #[tokio::test]
    async fn surfaces_api_errors() {
        let body = r#"{"success":false,"errors":[{"code":10102,"message":"permission denied"}],"messages":[],"result":null}"#;
        let response: &'static str = Box::leak(
            format!(
                "HTTP/1.1 403 Forbidden\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            )
            .into_boxed_str(),
        );
        let (url, handle) = capture_one_request(response).await;

        let svc = CloudflareEmailService::with_send_url(
            url,
            "tok-123".into(),
            "noreply@ololo.dev".into(),
        );
        let err = svc
            .send_email("user@example.com", "Hi", "<b>hi</b>", "hi")
            .await
            .expect_err("must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("10102"),
            "error should carry the CF code: {msg}"
        );
        assert!(msg.contains("permission denied"), "error detail: {msg}");
        handle.await.unwrap();
    }
}
