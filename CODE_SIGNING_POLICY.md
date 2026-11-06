# Glass Mixer — Code Signing Policy

## Status

**Application pending. Code signing is not active yet.**

Glass Mixer releases published before acceptance into the SignPath Foundation program are not Authenticode-signed.

The project intends to use SignPath Foundation for free code signing of official Windows releases, subject to application approval and successful integration.

## Signing Provider

Free code signing provided by SignPath.io, certificate by SignPath Foundation.

Once the application is approved and signing is integrated, official Windows release binaries will be signed using the SignPath Foundation certificate.

Windows will identify SignPath Foundation as the certificate publisher.

## What Will Be Signed

The intended signing scope includes:

- The official Glass Mixer Windows executable.
- The official Windows installer.
- Other eligible project-owned Windows executables, where applicable.

Only binaries built from Glass Mixer's own source code through its approved release process will be submitted for signing.

Third-party components will not be represented as Glass Mixer-authored or signed using the project's signing authorization.

## Build and Release Process

Official release binaries will be built using the project's GitHub Actions workflows from the public source repository.

Signing requests will be restricted to authorized release builds.

Every production release signing request will require manual approval in accordance with SignPath Foundation requirements.

Signing credentials and private keys will not be stored in the repository.

## Team Responsibilities

Glass Mixer is independently maintained.

- **Authors:** The project maintainer(s) with authorized source-code write access.
- **Reviewers:** The project maintainer and any designated contributors authorized to review proposed changes.
- **Approvers:** The authorized project maintainer responsible for reviewing and approving release-signing requests.

Where responsibilities overlap, the same maintainer may perform multiple roles, subject to SignPath Foundation approval and its applicable requirements.

## Privacy

This program will not transfer any information to other networked systems unless specifically requested by the user or the person installing or operating it.

Glass Mixer does not include application telemetry or analytics. Application settings and diagnostic logs are stored locally.

## Signature Verification

Once signing becomes active, users will be able to verify official Windows executable signatures through Windows File Properties → Digital Signatures.

Official releases will continue to provide SHA-256 checksums for distribution integrity verification.

## Changes to This Policy

This policy describes the intended signing process while the SignPath Foundation application is pending.

It will be updated when the application is approved and production signing becomes operational.
