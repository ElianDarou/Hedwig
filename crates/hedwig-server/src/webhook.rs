use crate::Database;
use hmac::{Hmac, Mac};
use reqwest::redirect::Policy;
use sha2::Sha256;
use std::time::Duration;

pub async fn deliver_pending_webhooks(database: &Database) -> anyhow::Result<usize> {
    let deliveries = database.due_webhook_deliveries(50)?;
    if deliveries.is_empty() {
        return Ok(0);
    }
    let client = reqwest::Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(15))
        .build()?;
    let mut delivered = 0;
    for delivery in deliveries {
        type HmacSha256 = Hmac<Sha256>;
        let mut mac = <HmacSha256 as Mac>::new_from_slice(delivery.signing_secret.as_bytes())?;
        mac.update(delivery.payload.as_bytes());
        let signature = format!("v1={}", hex::encode(mac.finalize().into_bytes()));
        let response = client
            .post(&delivery.url)
            .header("Content-Type", "application/json")
            .header("X-Cloud-Event-ID", &delivery.event_id)
            .header("X-Cloud-Signature", signature)
            .body(delivery.payload.clone())
            .send()
            .await;
        match response {
            Ok(response) if response.status().is_success() => {
                database.complete_webhook_delivery(&delivery, true, None)?;
                delivered += 1;
            }
            Ok(response) => database.complete_webhook_delivery(
                &delivery,
                false,
                Some(&format!("HTTP {}", response.status())),
            )?,
            Err(error) => {
                database.complete_webhook_delivery(&delivery, false, Some(&error.to_string()))?
            }
        }
    }
    Ok(delivered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_is_stable_for_retry() {
        type HmacSha256 = Hmac<Sha256>;
        let mut first = <HmacSha256 as Mac>::new_from_slice(b"secret").unwrap();
        first.update(br#"{"event_id":"same"}"#);
        let mut second = <HmacSha256 as Mac>::new_from_slice(b"secret").unwrap();
        second.update(br#"{"event_id":"same"}"#);
        assert_eq!(
            first.finalize().into_bytes(),
            second.finalize().into_bytes()
        );
    }
}
