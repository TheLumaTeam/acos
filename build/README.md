# ACOS image builder

This directory builds bootable, reproducible ARM64 ACOS disk images. It uses
a separately pinned Nixpkgs revision and carries a build-time snapshot of the
guest source under `guest/`, so the builder is a self-contained flake and does
not depend on impure paths outside its input.

## Requirements

- Nix with flakes enabled.
- An `aarch64-linux` builder, either local or configured as a Nix remote
  builder. The image contains an ARM64 Linux kernel and userspace; macOS cannot
  execute the Linux disk-image build derivations itself.
- Enough builder storage for the NixOS closure and a 16 GiB virtual disk.

## Build

From the repository root:

```sh
./build/acos-build
./build/acos-build --profile developer
```

The default `runtime` profile is intended for normal ACOS computers. The
`developer` profile adds the development toolchains and rootless container
tools already defined by `guest/nixos/configuration.nix`.

The `guest/` subtree is mirrored from the repository's top-level `guest/`
directory. When changing the guest OS or desktop, copy those changes into this
snapshot as well; keeping the image builder self-contained makes its Nix input
reproducible when invoked directly from `build/`.

Equivalent direct Nix commands:

```sh
nix build ./build#runtime
nix build ./build#developer
```

To choose the result link, pass `--out-link PATH` to the script or `--out-link`
to `nix build`. The output directory contains:

- `acos-PROFILE-aarch64.qcow2`: bootable EFI/QCOW2 disk image.
- `metadata.json`: schema version, profile, architecture, format, pinned
  Nixpkgs revision, branding name and virtual disk capacity.
- `qemu-img-info.json`: machine-readable QEMU image inspection output.
- `SHA256SUMS`: SHA-256 digest of the disk image.

The metadata is deterministic for a given source revision and profile. Nix
content addressing provides the build cache; the checksum allows downloaded
images to be checked independently.

## Profiles and branding

Profile definitions live in `profiles/`. They currently select the desktop
agent display name. The shared mark is `branding/acos-mark.svg`; the NixOS
desktop module installs it as `/etc/acos/desktop/agent-logo`. To change the
logo, replace that SVG while keeping it a self-contained image. The runtime
and developer profile package sets remain defined by the guest configuration.

## Integration with the root flake

The root `flake.nix` can expose these builds later without changing the guest
VM development outputs. Add the build flake as a locked local input:

```nix
inputs.acos-build.url = "path:./build";
```

Add `acos-build` to the root flake's `outputs` arguments, then add:

```nix
packages = forAllSystems (system:
  nixpkgs.lib.optionalAttrs (system == "aarch64-linux") {
    acos-image = acos-build.packages.${system}.runtime;
    acos-image-developer = acos-build.packages.${system}.developer;
  });
```

Run `nix flake lock` after adding the input. Keep image builds on an ARM64 Linux
builder; the existing macOS HVF VM workflow continues to use the root flake's
`system.build.vm` output.
