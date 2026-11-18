# Cortex 0.2.2 for Linux

The latest Linux release is Cortex 0.2.2. See the [Cortex release list](https://github.com/amad-malik/ai-cortex/releases).

## Install

Download the installer over HTTPS and run the saved file:

```sh
curl --proto '=https' --tlsv1.2 -fsSLo install.sh https://github.com/amad-malik/ai-cortex/releases/download/v0.2.2/install.sh && sh ./install.sh --project cortex-1 --lead lead1 --release 0.2.2 --base-url https://github.com/amad-malik/ai-cortex/releases/download/v0.2.2 --yes
```

## What changed

- The installer ends with one summary: what was done, the test result (health, containers, sign-in) and where the token file is.
- Running the same install command again is safe: it reports each step as already done and changes nothing.

## Known limit in 0.2.2

If the installer's standard output or standard error is a pipe that nobody reads and the
pipe fills, a stop or a signal waits until the pipe is read or closed. No credential is
exposed while it waits. Until the fix planned for the next release, do not pipe the
installer into a program that stops reading; send its output to the terminal or to a
file. If you end such a wait with kill -9 or by closing the pipe, the next install may
stop with a stale mint reservation: first check that no installer is running, then
remove that reservation directory as the message says (it names the directory, under
/run/user/ followed by your user id, and shows its owner record). If the install stopped
while it was creating the token and no new token file was written, run the installer
again with --reissue-token.

## Built with

Both programs are built with the Rust toolchain 1.99.1 (the toolchain file of the source commit); a Rust toolchain includes its standard library in the programs it builds, so both programs are expected to contain code of it (core, alloc, std and its
runtime pieces).

- [PostgreSQL](https://www.postgresql.org/) + [pgvector](https://github.com/pgvector/pgvector): `docker.io/pgvector/pgvector:0.8.2-pg18`.
- Operating-system packages: [podman](https://podman.io/), [podman-compose](https://github.com/containers/podman-compose).
- [Rocky Linux 10.2 Red Quartz](https://rockylinux.org): `quay.io/rockylinux/rockylinux`.

## Licence and third-party notices

**MIT.** The Cortex application is distributed under the MIT licence.
The MIT licence covers the application as we distribute it; the components it contains and the packages it installs keep their own
licences, which the release's notices file records.

See `LICENSE` and `THIRD-PARTY-NOTICES.txt`.

## Not in this release

Homebrew is not available in v0.2.2.
