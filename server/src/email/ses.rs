//! Production AWS SES v2 email service implementation.

use super::{EmailError, EmailService};
use async_trait::async_trait;
use aws_sdk_sesv2::{
    Client,
    types::{Body, Content, Destination, EmailContent, Message},
};

/// Production [`EmailService`] backed by AWS SES v2.
///
/// The [`Client`] is constructed once at startup and stored in [`AppState`]
/// (NFR-004: connection reuse via SDK's internal HTTP pool).
pub struct SesEmailService {
    client: Client,
    from_address: String,
}

impl SesEmailService {
    /// Create a new [`SesEmailService`] from a pre-built AWS SES client.
    ///
    /// The client must be constructed with the appropriate credentials and
    /// region before being passed here.
    pub fn new(client: Client, from_address: String) -> Self {
        Self {
            client,
            from_address,
        }
    }
}

#[async_trait]
impl EmailService for SesEmailService {
    async fn send_email(
        &self,
        to: &str,
        subject: &str,
        body_html: &str,
        body_text: &str,
    ) -> Result<(), EmailError> {
        let dest = Destination::builder().to_addresses(to).build();

        let subject_content = Content::builder()
            .data(subject)
            .charset("UTF-8")
            .build()
            .map_err(|e| EmailError::SendFailure(e.to_string()))?;

        let html_content = Content::builder()
            .data(body_html)
            .charset("UTF-8")
            .build()
            .map_err(|e| EmailError::SendFailure(e.to_string()))?;

        let text_content = Content::builder()
            .data(body_text)
            .charset("UTF-8")
            .build()
            .map_err(|e| EmailError::SendFailure(e.to_string()))?;

        let body = Body::builder()
            .html(html_content)
            .text(text_content)
            .build();

        let message = Message::builder()
            .subject(subject_content)
            .body(body)
            .build();

        let email_content = EmailContent::builder().simple(message).build();

        self.client
            .send_email()
            .from_email_address(&self.from_address)
            .destination(dest)
            .content(email_content)
            .send()
            .await
            .map_err(|e| EmailError::SendFailure(e.to_string()))?;

        Ok(())
    }
}
