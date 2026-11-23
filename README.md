# ACOS — Agent Computers Operating System

ACOS is an open computer platform for AI agents and people. It provides a stable API for managing Linux virtual machines and OCI containers across local and cloud hosts.

## Project status

The host now includes a loopback-first, bearer-authenticated HTTP API, a SQLite-backed computer registry, and a QEMU VM lifecycle backend. QEMU Guest Agent is connected over a private virtio-serial channel for command execution as the guest `agent` user, workspace file transfer and guest OS/filesystem information. OCI containers, live streamed command output, detailed resource telemetry, snapshots, remote-host auth/TLS and event streaming remain in development.

## Development on macOS (Apple Silicon)

Requirements: macOS 27 or later, Apple Silicon, Rust stable, and Nix with flakes enabled. The Rust host core builds natively on macOS ARM64. Linux guests run directly on the Mac with QEMU's Apple HVF hardware accelerator and native Cocoa display. OCI containers use the guest's Linux kernel.

```sh
nix develop
cargo test --workspace
cargo run -p acos-host
```

Build the standalone host executable with `cargo build --release -p acos-host`,
or install it into Cargo's user binary directory with `just install`. The
Rust executable builds natively for macOS ARM64 and Linux, allowing agent
harnesses on those hosts to invoke the same CLI. Use `acos-host --json` for machine-readable status and
`acos-host --help` for command-line options.

## ACOS Host API

Build and start the host:

```sh
cargo build --release -p acos-host
export ACOS_IMAGE_DIR="$HOME/.local/share/acos/images"
export ACOS_DATA_DIR="$HOME/.local/share/acos"
mkdir -p "$ACOS_IMAGE_DIR"
acos-host serve
```

Set `ACOS_API_TOKEN` before starting to use a stable secret; otherwise the host
generates one and prints it once. The API binds to `127.0.0.1:8787` by default.
`ACOS_LISTEN` can override the bind address; do not expose it to a network
without a TLS-terminating proxy and a properly managed secret. Put bootable
ARM64 QCOW2 base images in `ACOS_IMAGE_DIR`. Create requests refer to a filename
inside that directory, never an arbitrary host path. Each VM receives a private
QCOW2 overlay and QMP/QGA sockets under `ACOS_DATA_DIR/computers/`.

```sh
curl http://127.0.0.1:8787/v1/capabilities
curl -H "Authorization: Bearer $ACOS_API_TOKEN" \\
  http://127.0.0.1:8787/v1/computers
```

The API currently supports create/list/inspect/start/stop/pause/resume/delete,
guest command execution, workspace file transfer, and guest OS/hostname/filesystem
information. File transfer is restricted to workspace-relative paths and 10 MiB.
Commands use argv (never implicit shell parsing), run as the guest `agent` account,
and have bounded output and a timeout. The SQLite database is stored at
`$ACOS_DATA_DIR/acos.db`.

## ACOS Guest (ARM64)

The ARM64 guest defaults to a compact `runtime` profile with Bash, Python, Git, network/file tools, Chromium, and the ACOS desktop. Nix flakes are enabled so additional tools can be installed on demand. The optional `developer` profile includes the full prebuilt toolchain set (Node.js, Perl, Ruby, Rust, PostgreSQL, Redis, and rootless Podman/Buildah/Skopeo); build it with `nix build .#nixosConfigurations.acos-aarch64-developer.config.system.build.vm`.

The graphical session is ACOS's own minimal shell: black background, the image-builder-provided agent logo and name at the top, and a centered launcher populated from installed `.desktop` applications. Labwc supplies only Wayland window management; there is no KDE, GNOME, Xorg, or display manager. Chromium is reused as the shell renderer and Foot provides a small terminal.

Image builders can set `acos.desktop.agentName` and `acos.desktop.agentLogo` in their NixOS module to brand the shell. The logo accepts a PNG, JPEG, WebP, or SVG image; without one, the shell displays a simple initial.

The interactive `agent` account has passwordless sudo **inside the guest** so an agent can install packages and manage its computer; this does not grant access to the host. The guest publishes `/etc/acos/agent.json` as its machine-readable capability manifest. Run `acos-info` or `acos-info --json` from the guest to inspect it.

The working directory is `/workspace`, backed by `~/.local/share/acos/workspace` on the Mac and shared into the guest through QEMU's writable 9p filesystem device. The HVF VM uses 2 vCPUs and 2 GiB RAM. Its persistent 16 GiB QCOW2 disk is `~/.local/share/acos/qemu/acos-data.qcow2`; the compressed, read-only Nix store image is cached under `~/.local/share/acos/qemu/runtime-<system-hash>/`.

Evaluate the guest configuration with `just guest-eval` and build it with `just guest-build`.

Boot the VM directly on macOS with `just guest-run`. QEMU uses Apple's HVF accelerator and opens a native Cocoa window with a Virtio GPU. The serial console remains attached to the terminal. On Linux, the same recipes use QEMU/KVM when available.

The guest enables QEMU Guest Agent. ACOS Host connects to it over a per-VM
virtio-serial socket; no guest-agent TCP port is exposed.

## Image builder

Build a reproducible ARM64 QCOW2 image with `./build/acos-build` or
`./build/acos-build --profile developer`. See [build/README.md](build/README.md)
for builder requirements, output metadata and checksums.

## Layout

- `docs/` architecture and development constraints
- `spec/` versioned public API contract
- `host/` backend-independent host core and service
- `guest/` future NixOS-based ACOS Guest profiles
- `backends/` future platform VM and OCI implementations
- `build/` image-building configuration
- `sdk/` Python and TypeScript client libraries
- `tests/` cross-component and conformance tests

## License

Apache-2.0. See `LICENSE`.
