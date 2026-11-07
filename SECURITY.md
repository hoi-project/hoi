# Security policy

hoi is preview software. Compatibility may change and data may be lost. There is no
independent security audit or supported stable release yet.

Report vulnerabilities through this repository's **Security -> Report a vulnerability**
page when private reporting is available. Include the affected version, reproduction
steps and impact. If that option is unavailable, write to
[hello@hoi.sh](mailto:hello@hoi.sh) and never disclose details in a public issue. Do not include
real private keys, personal data or illegal material in reports.

## Current limitations

- Recipients can retain copies after posts expire.
- A public node can read public records it stores. Private-relay encryption does not
  hide all connection metadata or prevent withholding.
- A compromised client distribution can expose data and keys available to that client.
- Recovery keys restore identity; they do not restore all local data or private-group state.
- Registry discovery depends on its operator. Keep independent copies and full invites.

Review the relevant deployment guide before operating a service. Report security issues
even when they concern a documented limitation with an unexpected impact.
