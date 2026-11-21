## Safer TLS checks

- **Dependency updates fix eleven published advisories.** The certificate
  and endpoint checks connect through rustls, and v0.10.0 shipped versions
  of rustls, rustls-webpki and webpki-roots with known flaws, including
  certificate verification that could accept distrusted intermediates and
  a ServerHello downgrade to classical key exchange. All dependencies are
  now on their latest releases, and `cargo audit` reports no
  vulnerabilities.

## Binaries built in CI

- **Releases are built and published by GitHub Actions.** Each release now
  ships `checksums.txt`, an SPDX SBOM per binary, and build-provenance
  attestations. Verify a download with
  `gh attestation verify ./alertpaca --repo fabioconcina/alertpaca`.

- **Runs on older Linux.** The binaries now need glibc 2.17 or newer, down
  from 2.30, so they also run on distributions like RHEL 7 and Ubuntu
  18.04.

- **Fixed download links in the README.** The macOS link pointed at a
  binary that was never published; it now points at the Linux ARM64 build.
  On macOS, build from source.

- **Building from source needs Rust 1.95 or newer.**
