#!/usr/bin/env bash
# Create the SES sending identity for the newsletter subdomain and print the
# DNS records it needs.
#
# The subdomain is deliberate: newsletter reputation is kept away from the
# transactional domain, so a bad campaign cannot cost us password-reset mail.
#
# Run after `aws login`. Safe to re-run — creating an existing identity is
# reported and skipped.
set -euo pipefail

DOMAIN="${SES_DOMAIN:-news.ololo.dev}"
REGION="${AWS_REGION:-us-east-1}"

echo "==> identity for $DOMAIN in $REGION"
if aws sesv2 get-email-identity --email-identity "$DOMAIN" --region "$REGION" >/dev/null 2>&1; then
  echo "    already exists"
else
  aws sesv2 create-email-identity \
    --email-identity "$DOMAIN" \
    --dkim-signing-attributes NextSigningKeyLength=RSA_2048_BIT \
    --region "$REGION" >/dev/null
  echo "    created"
fi

echo
echo "==> DNS records to add (Cloudflare, proxy OFF — these are not web traffic)"
aws sesv2 get-email-identity --email-identity "$DOMAIN" --region "$REGION" \
  --query 'DkimAttributes.Tokens' --output text 2>/dev/null \
  | tr '\t' '\n' | while read -r token; do
      [ -n "$token" ] && printf '  CNAME  %s._domainkey.%s  ->  %s.dkim.amazonses.com\n' \
        "$token" "$DOMAIN" "$token"
    done

cat <<TXT

  TXT    $DOMAIN                     ->  "v=spf1 include:amazonses.com ~all"
  TXT    _dmarc.$DOMAIN              ->  "v=DMARC1; p=none; rua=mailto:${SES_CONTACT_EMAIL:-postmaster@$DOMAIN}; fo=1"

  MX     $DOMAIN  (optional, for bounce return-path alignment)
                                     ->  10 feedback-smtp.$REGION.amazonses.com

Verification is not instant. Check with:
  aws sesv2 get-email-identity --email-identity $DOMAIN --region $REGION \\
    --query '{Verified:VerifiedForSendingStatus,Dkim:DkimAttributes.Status}'
TXT
