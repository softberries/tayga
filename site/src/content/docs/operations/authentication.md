---
title: "Authentication"
description: "Turning on the login page and protecting the API."
---

Authentication is off by default. Turning it on in the `[auth]` section adds a login page and protects every `/api/*` route except a few open ones. There is one account, whose password is stored as an Argon2id hash made with `tayga-devtools hash-password`. Sessions are signed cookies; scripts can use HTTP Basic credentials instead. Password checks are limited to 5 attempts per client IP in 5 minutes.

:::caution[TODO: this page is being written]
It will cover:

- Every `[auth]` setting with its default.
- Making the password hash, and the `$` pitfall in Compose files.
- Sessions, sign-out, the rate limit and HTTPS behind a reverse proxy.
- Which routes stay open.
:::
