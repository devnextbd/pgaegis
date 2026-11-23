# pgaegis supported platforms and qualified dependency baselines

Last reviewed: 2026-11-24

pgaegis separates **package compatibility** from **production HA certification**. A package may install successfully on an operating system without that exact OS/topology being certified for production HA.

## Primary supported operating systems

| Platform | Support level | Current baseline | Notes |
|---|---|---:|---|
| Debian | Tier 1 / production reference | Debian 13.x (current point release: 13.8, trixie) | Primary PostgreSQL/HA/PITR/storage qualification platform |
| Ubuntu | Tier 1 package support | Ubuntu 26.04 LTS (current point release: 26.04.1 LTS) | Native `.deb` lifecycle CI; HA certification remains Debian-reference until a full Ubuntu HA qualification lane passes |
| Fedora | Package-qualified | Fedora 45 | Native `.rpm` lifecycle CI |
| Rocky Linux | Package-qualified | Rocky Linux 10 | Native `.rpm` lifecycle CI |
| AlmaLinux | Package-qualified | AlmaLinux 10 | Native `.rpm` lifecycle CI |
| openSUSE | Package-qualified | openSUSE Leap 16.1 | Native `.rpm` lifecycle CI |
| Arch Linux | Package-qualified rolling target | current Arch | Native `.pkg.tar.zst` lifecycle CI |
| Other Linux | Generic compatibility | Alpine used as generic CI target | Static `.tar.gz` fallback |

Architectures: `x86_64` and `aarch64`.

## Toolchain baseline

| Component | Qualified baseline |
|---|---:|
| Rust | 1.100.0 stable |
| Node.js for GitHub Actions project steps | 26.13.1 LTS |
| GitHub checkout action | actions/checkout@v7 |
| GitHub setup-node action | actions/setup-node@v7 |
| GitHub upload-artifact action | actions/upload-artifact@v7 |

Rust has no LTS channel. pgaegis uses the current stable Rust toolchain and pins it for reproducible qualification.

## PostgreSQL / HA stack baseline

| Component | Qualified/selected baseline | Policy |
|---|---:|---|
| PostgreSQL | 18.7 | Current supported minor of the PostgreSQL 18 production baseline |
| Patroni | 4.1.5 | Current latest Patroni release |
| etcd | 3.7.4 | Current latest etcd release |
| pgBackRest | 2.60.1 | Current stable release |
| PgBouncer | distro-current in qualification images | Upgrade only after routing lifecycle qualification passes |
| HAProxy | distro-current in qualification images | Upgrade only after routing lifecycle qualification passes |

PostgreSQL 19.0 is the newest PostgreSQL major release, but it is **not yet a pgaegis production-certified baseline**. A PostgreSQL major version is not upgraded merely because it is newer. It must first pass Patroni compatibility, HA failover, pg_rewind, PITR, routing, package, and storage-fault qualification.

## Dependency update policy

pgaegis uses **latest stable compatible and qualified**, not unqualified floating versions.

1. Security/patch releases inside the qualified major line should be adopted promptly.
2. Critical control-plane versions are pinned for reproducible CI.
3. A new upstream major/minor must pass the full qualification suite before becoming the production baseline.
4. `latest`, beta, RC, nightly, alpha, or unreleased tags are never production defaults.
5. Release documentation must identify the exact qualified versions.
