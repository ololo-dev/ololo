#!/usr/bin/env bash
# Wire SES bounce and complaint notifications to our webhook.
#
# Without this the list rots silently: dead mailboxes stay on it and get
# mailed again next month, and SES suspends senders over roughly 5% bounces
# or 0.1% complaints. It is also the section AWS reads hardest on a
# production-access request.
#
# Needs SES_NOTIFICATION_SECRET — the same value set in Settings -> Email.
set -euo pipefail

DOMAIN="${SES_DOMAIN:-news.ololo.dev}"
REGION="${AWS_REGION:-us-east-1}"
ENDPOINT="${SES_WEBHOOK_BASE:-https://ololo.dev}/api/email/ses-notifications"
: "${SES_NOTIFICATION_SECRET:?set SES_NOTIFICATION_SECRET to the value stored in Settings -> Email}"

TOPIC_NAME="ololo-ses-feedback"

echo "==> SNS topic $TOPIC_NAME"
TOPIC_ARN=$(aws sns create-topic --name "$TOPIC_NAME" --region "$REGION" \
  --query TopicArn --output text)
echo "    $TOPIC_ARN"

echo "==> subscribing the webhook"
# The secret is the entire gate: SNS cannot authenticate itself, and anyone
# who learns this URL can unsubscribe every address on the list.
aws sns subscribe \
  --topic-arn "$TOPIC_ARN" \
  --protocol https \
  --notification-endpoint "${ENDPOINT}?token=${SES_NOTIFICATION_SECRET}" \
  --region "$REGION" >/dev/null
echo "    subscribed; the server auto-confirms the handshake"

echo "==> pointing SES at the topic"
for kind in Bounce Complaint; do
  aws ses set-identity-notification-topic \
    --identity "$DOMAIN" --notification-type "$kind" --sns-topic "$TOPIC_ARN" \
    --region "$REGION"
  echo "    $kind -> topic"
done

# Bounces and complaints must not also arrive by email, or they land in an
# inbox nobody reads instead of in the database.
for kind in Bounce Complaint; do
  aws ses set-identity-feedback-forwarding-enabled \
    --identity "$DOMAIN" --no-forwarding-enabled --region "$REGION" 2>/dev/null || true
done

echo
echo "Confirm the subscription went through:"
echo "  aws sns list-subscriptions-by-topic --topic-arn $TOPIC_ARN --region $REGION \\"
echo "    --query 'Subscriptions[].[Protocol,SubscriptionArn]' --output table"
echo "A SubscriptionArn of 'PendingConfirmation' means the server did not"
echo "answer — check the secret matches Settings -> Email."
