#!/usr/bin/env bash
# Submit the SES production access request.
#
# Run this LAST. Submitting before the domain verifies and before bounce
# handling is live is how the previous request was denied — a reviewer checks
# whether the described setup actually exists.
set -euo pipefail

REGION="${AWS_REGION:-us-east-1}"
DOMAIN="${SES_DOMAIN:-news.ololo.dev}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "==> preflight"
STATUS=$(aws sesv2 get-email-identity --email-identity "$DOMAIN" --region "$REGION" \
  --query 'VerifiedForSendingStatus' --output text 2>/dev/null || echo "MISSING")
if [ "$STATUS" != "True" ]; then
  echo "    $DOMAIN is not verified for sending (status: $STATUS)."
  echo "    Run 01-verify-domain.sh, add the DNS records, and wait for verification."
  exit 1
fi
echo "    $DOMAIN verified"

aws sesv2 put-account-details \
  --region "$REGION" \
  --mail-type MARKETING \
  --website-url "https://ololo.dev" \
  --contact-language EN \
  --use-case-description "file://${HERE}/use-case.txt" \
  --additional-contact-email-addresses "${SES_CONTACT_EMAIL:?set SES_CONTACT_EMAIL}" \
  --production-access-enabled

echo
echo "Submitted. Track it in Support Center; SES usually answers within 24h."
echo "Check status with:"
echo "  aws sesv2 get-account --region $REGION --query '{Prod:ProductionAccessEnabled,Sending:SendingEnabled}'"
