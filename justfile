default:
    @just --list

test:
    cargo test --workspace

check:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace

run:
    cargo run -p acos-host

guest-eval:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ "$(uname -s)" == Darwin ]]; then
        nix eval .#nixosConfigurations.acos-aarch64.config.system.build.toplevel.drvPath
    else
        nix eval .#nixosConfigurations.acos-aarch64-qemu.config.system.build.toplevel.drvPath
    fi

guest-build:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ "$(uname -s)" == Darwin ]]; then
        nix build .#nixosConfigurations.acos-aarch64.config.system.build.vm
    else
        nix build .#nixosConfigurations.acos-aarch64-qemu.config.system.build.vm
    fi

guest-run:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ "$(uname -s)" == Darwin ]]; then
        state="$HOME/.local/share/acos/qemu"
        workspace="$HOME/.local/share/acos/workspace"
        system_path="$(nix eval --raw .#nixosConfigurations.acos-aarch64.config.system.build.toplevel)"
        system_hash="${system_path#/nix/store/}"
        system_hash="${system_hash%%-*}"
        runtime="$state/runtime-$system_hash"
        mkdir -p "$state" "$workspace" "$runtime"
        export NIX_DISK_IMAGE="$state/acos-data.qcow2"
        export TMPDIR="$runtime"
        export USE_TMPDIR=1
        export ACOS_STORE_IMAGE_CACHE=1
        nix build .#nixosConfigurations.acos-aarch64.config.system.build.vm
        exec ./result/bin/run-acos-vm
    else
        state="$HOME/.local/share/acos/qemu"
        workspace="$HOME/.local/share/acos/workspace"
        mkdir -p "$state" "$workspace"
        export NIX_DISK_IMAGE="$state/acos-data.qcow2"
        nix build .#nixosConfigurations.acos-aarch64-qemu.config.system.build.vm
        exec ./result/bin/run-acos-vm
    fi

install:
    cargo install --path host/acos-host

[windows]
guest-eval:
    wsl.exe -- bash -lc 'cd "$(wslpath -u "{{justfile_directory()}}")" && just guest-eval'

[windows]
guest-build:
    wsl.exe -- bash -lc 'cd "$(wslpath -u "{{justfile_directory()}}")" && just guest-build'

[windows]
guest-run:
    wsl.exe -- bash -lc 'cd "$(wslpath -u "{{justfile_directory()}}")" && just guest-run'
