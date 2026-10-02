# Security policy

## Supported version

Security fixes are targeted at the latest published release. Older versions do not have a separate maintenance commitment. Update from [GitHub Releases](https://github.com/gitfudge0/pinacora/releases).

## Report a vulnerability

Use GitHub's private [Report a vulnerability](https://github.com/gitfudge0/pinacora/security/advisories/new) form when available. Include affected versions, a description, reproduction steps or a minimal proof of concept, and the likely impact. Do not submit secrets, private gallery data, or personal information.

If the private form is unavailable, open a public issue asking the [maintainer](https://github.com/gitfudge0) for a private reporting channel, without disclosing the vulnerability or exploit details. There is no guaranteed response time or bug bounty program.

## Scope

Relevant reports include unsafe image handling, untrusted URL handling, filesystem/cache behavior, wallpaper application, and release artifact integrity. The gallery and its CDN are independent services; issues in those services should be reported to their operators.

Release app bundles are ad-hoc signed and are not Apple-notarized. Download only from this repository's releases. Follow macOS security prompts and the installation guidance in the README.
