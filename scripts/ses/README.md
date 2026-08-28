# SES production access for the newsletter

Run in order. Each step is safe to re-run.

Everything here needs an authenticated session — `aws login` — which is why
none of it is automated end to end.

## 0. Before anything

- The newsletter code must be **in production**. It shipped after `v0.8.0`,
  so `ololo.dev` does not have it yet: `/api/email/ses-notifications` returns
  404 there, and step 2 subscribes SNS to a URL that cannot answer. Cut a tag
  first.
- Set the webhook secret in **Settings → Email → Bounce & complaint webhook
  secret**. Step 2 needs the same value in `SES_NOTIFICATION_SECRET`.

## 1. Verify the sending domain

```sh
./01-verify-domain.sh
```

Creates the `news.ololo.dev` identity and prints the DNS records. Add them in
Cloudflare with the proxy **off** — they are mail records, not web traffic.
Verification takes minutes to a few hours.

A subdomain separate from `ololo.dev` is deliberate: a bad campaign should
not be able to cost us password-reset delivery.

## 2. Wire bounces and complaints

```sh
SES_NOTIFICATION_SECRET='<the value from Settings → Email>' ./02-bounce-notifications.sh
```

Creates an SNS topic, subscribes our webhook, points SES Bounce and Complaint
notifications at it, and turns off email feedback forwarding so the reports
land in the database rather than in an inbox nobody reads.

The server confirms the SNS handshake itself. If the subscription stays at
`PendingConfirmation`, the secret does not match.

## 3. Request production access

```sh
./03-request-production-access.sh
```

Refuses to submit until the domain is verified. Submitting before the
described setup exists is how the previous request was denied — a reviewer
checks.

The text lives in `use-case.txt`. It describes only what is actually built;
if the implementation changes, change the text with it.

## Afterwards

Sandbox lifts to 50k/day at 14/second. Watch the two numbers that get a
sender suspended:

```sh
aws sesv2 get-account --region us-east-1 \
  --query '{Prod:ProductionAccessEnabled,Sending:SendingEnabled,Quota:SendQuota}'
```

Bounce rate above 5% or complaints above 0.1% put the account under review.
Both are visible per-subscriber in Settings → Newsletter, as the `bounced` and
`complained` statuses.

## Known gap

`List-Unsubscribe` is not set. `EmailService::send_email` takes no headers,
and SES needs raw MIME to add them. Not required for approval, but Gmail
weights it heavily for bulk senders — worth doing before volume grows.
