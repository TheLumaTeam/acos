# Development environment

## Native macOS work

The host model, API validation, CLI, SDK generation and documentation checks can run on macOS ARM64. Use `nix develop` to enter the pinned Nix development shell, then `just check` for Rust formatting, linting and tests.

## Linux guest

The Mac boots the ARM64 Linux guest directly with QEMU's HVF accelerator, which uses Apple's Hypervisor.framework, and opens a native Cocoa display window with a Virtio GPU. The workspace is shared through QEMU's writable 9p filesystem device. Linux hosts use QEMU/KVM. The optional developer profile provides Podman inside the guest, where it has a native Linux kernel.

KVM is a Linux kernel facility and is unavailable on macOS. QEMU's HVF accelerator provides hardware virtualization directly on Apple Silicon, so the Mac does not need KVM or a nested Linux VM to boot ACOS.

On Windows, the Rust recipes (`just test`, `just check`, `just run` and `just install`) use the native Windows toolchain. The `guest-eval`, `guest-build` and `guest-run` recipes delegate to WSL2; install Nix and `just` inside that WSL distribution and keep the checkout in a path accessible from WSL.

## ACOS Guest

The ARM64 NixOS guest is defined by `guest/nixos/configuration.nix`. On Apple Silicon, `.#nixosConfigurations.acos-aarch64` uses QEMU with Apple's HVF accelerator and Cocoa display. The host QEMU build includes HVF and Cocoa support; the guest receives a Virtio GPU and USB input devices. On Linux, `.#nixosConfigurations.acos-aarch64-qemu` uses QEMU/KVM when available. `just guest-eval`, `just guest-build` and `just guest-run` select the appropriate backend for the host.

On macOS, `just guest-run` builds the VM on the Mac and boots it with HVF. The graphical display opens in a native Cocoa window; the serial console is attached to the terminal. The persistent QCOW2 disk lives at `~/.local/share/acos/qemu/acos-data.qcow2`, and `/workspace` is shared from `~/.local/share/acos/workspace` through QEMU's 9p filesystem device. On Linux, the QEMU launcher runs locally and uses KVM when available. Both hosts keep the writable guest store on the persistent disk.

The default `runtime` profile is the small downloadable computer runtime: Bash, Python, Git, common file/network tools, Chromium, and the ACOS desktop. Nix command/flakes support is enabled so additional toolchains can be installed when needed. The optional `developer` profile restores the full prebuilt toolchain set, including Node.js, Rust, Ruby, Perl, PostgreSQL, Redis, and rootless Podman/Buildah/Skopeo. Build it with `nix build .#nixosConfigurations.acos-aarch64-developer.config.system.build.vm`. `acos-info --json` returns `/etc/acos/agent.json`, a capability manifest that reflects the selected profile.

The graphical session starts automatically on tty1. Its Labwc autostart selects a 1440×900 mode on the connected Virtio display (falling back to 1280×800), so the QEMU window opens at a usable size. ACOS's custom launcher is in `guest/desktop/`; it discovers installed graphical `.desktop` entries, resolves their freedesktop icons, and launches them as the `agent` user. The launcher presents app icons, names, and descriptions in a desktop-style grid. Labwc is only the Wayland compositor/window manager. The shell uses Chromium for rendering and Foot as its terminal; no KDE/GNOME desktop packages, services or desktop environment are installed. Image builders can set `acos.desktop.agentName` and `acos.desktop.agentLogo` in a NixOS module.

The guest's `agent` account is available on the VM console or in the graphical session. Its passwordless sudo is confined to the guest; it does not grant host privileges. SSH is disabled.

The QEMU backend keeps its persistent disk separate from the project checkout. Stop the VM from the guest with `sudo poweroff`, or use Ctrl-A followed by `x` in QEMU's monitor. The guest's writable Nix store and home directory survive restarts, while `/workspace` remains a separate host share. The 9p share uses mapped extended-attribute ownership; the guest reapplies ownership once per boot and uses a group-writable `0002` umask so files created later remain editable by `agent`, without making the host workspace world-writable. On macOS, the compressed EROFS store image is cached by the NixOS system closure hash, so unchanged guests do not rebuild it at every launch. When rebuilt, `pv` displays incoming bytes and throughput; after input finishes, the wrapper explicitly reports that EROFS compression/finalization is continuing and emits a heartbeat every 10 seconds. Zstandard level 6 with 64 KiB clusters avoids the extreme CPU cost of level 22 while retaining useful compression. The cache marker is versioned so images made with earlier compression settings are rebuilt once.

## Current machine observations

- Apple Silicon ARM64, Apple M3 family.
- 8 GiB unified memory.
- macOS 27.2.1.
- Rust 1.100.0 and Cargo 1.100.0 available.
- Nix 2.34.9 available.
- Apple Hypervisor.framework (HVF) is available.
- QEMU is provided by the Nix host package set with HVF and Cocoa support; no separate installation is needed.
- Docker CLI is present; its daemon/runtime availability must be checked separately before OCI integration.

The ACOS repository does not install system-level tools or mutate host settings. Optional remote-builder configuration is managed separately in the machine's nix-darwin flake.
