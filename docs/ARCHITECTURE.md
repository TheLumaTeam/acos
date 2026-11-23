# ACOS architecture

## Product boundary

ACOS has two products with a versioned boundary between them:

- **ACOS Host** owns compute resources, storage, networking, identity, policy, backend selection and the public Computer API. It can run on macOS for development and on Linux for production.
- **ACOS Guest** is the Linux operating system installed in virtual machines. It provides normal Linux userspace, package management, an interactive agent account and QEMU Guest Agent over a private virtio-serial channel.

An OCI container is a process and filesystem isolation environment that shares its host Linux kernel. It is a `Computer` for common lifecycle and execution operations, but it is never advertised as a separate guest kernel or operating system.

## Host layers

```text
Agent / Human / SDK / CLI
          |
   HTTPS + OpenAPI v1
          |
 Authentication -> Capability authorization -> Quotas/Audit
          |
     Computer service
          |
   Backend-neutral model
       /       \
  VM backend  OCI backend
       |          |
 Virtualization  Container runtime
       |
QEMU Guest Agent (VMs)
```

The public API owns stable resource identifiers, validation, authorization and consistent errors. Backend adapters translate requests into platform operations and report observed state. A backend reporting an uncertain state must produce `unknown`; the host must never silently turn an observation failure into `stopped`.

## Backend interfaces

The host core depends on narrow traits, not concrete hypervisors:

- `ComputerBackend`: create, inspect, start, stop, pause, resume and delete a computer.
- `CommandBackend`: execute a command with an explicit working directory, environment allowlist, timeout and output limit.
- `FileBackend`: bounded read, write, list and metadata operations.
- `SnapshotBackend`: capability discovery, create and restore snapshots.
- `EventSource`: structured lifecycle, resource and guest events.

Backends advertise capabilities. Unsupported operations return a stable `capability_not_supported` error. Snapshot support, GUI access and pause/resume are not assumed to exist for every runtime.

## Platform decisions

| Platform | VM implementation | OCI implementation | Test environment |
| --- | --- | --- | --- |
| macOS Apple Silicon | QEMU with Apple HVF acceleration, Cocoa display and Virtio GPU | Podman/Docker through a Linux VM | Native Rust tests; Linux integration tests inside a Linux VM |
| Linux ARM64 | QEMU/KVM initially | Podman or another OCI runtime | Native integration tests |
| Linux x86-64 | QEMU/KVM when added | Podman or another OCI runtime | CI and native integration tests |

KVM is a Linux kernel facility and is unavailable on macOS. Linux containers require a Linux kernel. macOS cannot load Linux kernel modules. QEMU's HVF accelerator uses Apple's Hypervisor.framework to run ARM64 guests with hardware acceleration. QEMU without HVF or KVM can fall back to TCG emulation but is expected to be slower.

## Guest design

The initial guest is built from a NixOS ARM64 configuration. It provides the Linux kernel, systemd, DHCP networking, a serial console, standard command-line tools and an interactive `agent` account. SSH is disabled. The minimal graphical session is an ACOS-owned HTML/CSS/JavaScript shell rendered by the already-installed Chromium browser, running under the small Labwc Wayland compositor. Its black screen places the build-time agent logo and name at the top and a centered application launcher in the middle. Labwc handles windows only; ACOS supplies the desktop UI. No KDE, GNOME, Xorg or display manager is installed. Image builders set `acos.desktop.agentName` and `acos.desktop.agentLogo` to brand the shell. The QEMU backend uses ARM64 UEFI firmware, private QCOW2 overlays, user-mode networking, and a virtio-serial QGA channel. QGA command execution is wrapped to run as the guest `agent` user with bounded output and a timeout; the channel is not exposed over TCP.

The guest agent communicates with the host over a per-VM virtio-serial channel that is private to the QEMU process. The host API authenticates callers with a bearer token. The guest agent has no authority to grant host capabilities or alter host policy.

## Security model

Every API request is associated with an authenticated principal. Authorization is evaluated against a scoped capability set and the target computer. Capabilities are explicit (for example `computer.read`, `computer.start`, `command.execute`, `files.write`, `desktop.capture`, `snapshot.restore`) and can be revoked. The host records principal, target, operation, decision, timestamp and result in an append-only audit stream.

The host process runs with the least privilege needed by its selected backend. Guest and container workloads receive resource quotas and explicit network policy. Secrets are passed through short-lived channels and are never placed in command arguments, guest metadata or audit payloads. The API binds to loopback by default during development; remote deployment requires TLS and configured authentication. The first API release uses one configured bearer token rather than multi-principal capability authorization; per-principal grants and audit records remain future work.

Containers share the Linux host kernel, so their isolation strength is bounded by the selected runtime and kernel configuration. Untrusted workloads should use VMs or a stronger sandbox backend when available.

## API compatibility

The HTTP API is rooted at `/v1`. The OpenAPI document in `spec/openapi/v1.yaml` is authoritative. Resource identifiers are opaque to clients. State changes are asynchronous in the general case: requests return the latest observed state and clients subscribe to events or poll the resource. API error responses use a stable machine-readable code and request ID.

## Development constraints

The supported developer machine is Apple Silicon macOS. Rust host logic and contract tests run natively. Nix provides reproducible tooling and Linux guest configurations. Linux-only build and integration stages execute in a Linux VM or CI. Keep the default workflow lightweight: one Linux VM at a time, no mandatory desktop guest, and no host OS changes, SIP changes or privileged kernel modifications.
