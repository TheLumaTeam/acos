{
  description = "ACOS development environment (macOS Apple Silicon and Linux)";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "aarch64-darwin" "aarch64-linux" "x86_64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      macHostPkgs = import nixpkgs { system = "aarch64-darwin"; };
      compressedErofsUtils = macHostPkgs.runCommand "erofs-utils-zstd-cache" { } ''
        mkdir -p $out/bin
        cat > $out/bin/mkfs.erofs <<'EOF'
        #!${macHostPkgs.runtimeShell}
        set -eu
        output=""
        tar_mode=0
        for arg do
          output="$arg"
          if [ "$arg" = "--tar=f" ]; then tar_mode=1; fi
        done
        cache_format="erofs-zstd-level6-64k-v4"
        if [ "''${ACOS_STORE_IMAGE_CACHE:-0}" = "1" ] && [ "$tar_mode" = "1" ] && [ -s "$output" ] && [ -f "$output.complete" ] && [ "$(cat "$output.complete")" = "$cache_format" ]; then
          echo "Reusing cached EROFS store image: $output" >&2
          exit 0
        fi
        rm -f "$output" "$output.complete"
        workers="$(sysctl -n hw.logicalcpu 2>/dev/null || echo 8)"
        args=()
        for arg do
          # qemu-vm.nix always supplies --quiet. Drop it so EROFS can report
          # useful diagnostics and its final image statistics.
          if [ "$arg" != "--quiet" ]; then args+=("$arg"); fi
        done

        temp_dir="$(mktemp -d "''${TMPDIR:-/tmp}/acos-erofs.XXXXXXXX")"
        fifo="$temp_dir/input"
        mkfifo "$fifo"
        cleanup() { rm -rf "$temp_dir"; }
        trap cleanup EXIT
        trap 'exit 130' INT
        trap 'exit 143' TERM

        echo "Packing Nix store (zstd level 6, 64 KiB clusters, $workers workers):" >&2
        ${macHostPkgs.erofs-utils}/bin/mkfs.erofs --workers="$workers" -z zstd,level=6 -C 65536 "''${args[@]}" < "$fifo" &
        mkfs_pid=$!
        set +e
        ${macHostPkgs.pv}/bin/pv --force --progress --timer --bytes --rate --average-rate --name "Nix store input" > "$fifo"
        pv_status=$?
        set -e
        echo "Nix store input complete; waiting for EROFS compression and image finalization..." >&2
        if [ "$pv_status" -ne 0 ]; then
          kill "$mkfs_pid" 2>/dev/null || true
          wait "$mkfs_pid" 2>/dev/null || true
          exit "$pv_status"
        fi
        (
          while :; do
            sleep 10
            echo "EROFS is still compressing/finalizing the Nix store image..." >&2
          done
        ) &
        heartbeat_pid=$!
        set +e
        wait "$mkfs_pid"
        mkfs_status=$?
        set -e
        kill "$heartbeat_pid" 2>/dev/null || true
        wait "$heartbeat_pid" 2>/dev/null || true
        if [ "$mkfs_status" -ne 0 ]; then exit "$mkfs_status"; fi
        printf '%s\n' "$cache_format" > "$output.complete"
        EOF
        chmod +x $out/bin/mkfs.erofs
      '';
      acosGuestMac = nixpkgs.lib.nixosSystem {
        system = "aarch64-linux";
        modules = [
          (nixpkgs + "/nixos/modules/virtualisation/qemu-vm.nix")
          ./guest/nixos/configuration.nix
          ({ lib, ... }: {
            virtualisation.host.pkgs = macHostPkgs // {
              erofs-utils = compressedErofsUtils;
            };
            virtualisation = {
              memorySize = 2048;
              cores = 2;
              diskSize = 16384;
              writableStoreUseTmpfs = false;
              graphics = true;
              useNixStoreImage = true;
              writableStore = true;
              mountHostNixStore = false;
              diskImage = "./acos-data.qcow2";
              qemu = {
                package = macHostPkgs.qemu;
                forceAccel = true;
                options = [
                  "-display cocoa"
                  "-device virtio-gpu-pci"
                  "-serial mon:stdio"
                  "-virtfs local,path=$HOME/.local/share/acos/workspace,security_model=mapped-xattr,mount_tag=shared"
                ];
              };
              sharedDirectories = lib.mkForce { };
              fileSystems."/tmp/shared" = {
                device = "shared";
                fsType = "9p";
                neededForBoot = true;
                options = [
                  "trans=virtio"
                  "version=9p2000.L"
                  "msize=16384"
                  "x-systemd.requires=modprobe@9pnet_virtio.service"
                  "access=any"
                ];
              };
            };
            boot.kernelParams = [ "console=ttyAMA0,115200" "console=tty0" ];
          })
        ];
      };
      acosGuestQemu = nixpkgs.lib.nixosSystem {
        system = "aarch64-linux";
        modules = [
          (nixpkgs + "/nixos/modules/virtualisation/qemu-vm.nix")
          ./guest/nixos/configuration.nix
          ({ lib, ... }: {
            virtualisation.vmVariant = {
              virtualisation.memorySize = 2048;
              virtualisation.cores = 2;
              virtualisation.diskSize = 16384;
              virtualisation.writableStoreUseTmpfs = false;
              virtualisation.sharedDirectories = lib.mkForce { };
              virtualisation.qemu.options = [
                "-virtfs local,path=$HOME/.local/share/acos/workspace,security_model=mapped-xattr,mount_tag=shared"
              ];
              virtualisation.fileSystems."/tmp/shared" = {
                device = "shared";
                fsType = "9p";
                neededForBoot = true;
                options = [
                  "trans=virtio"
                  "version=9p2000.L"
                  "msize=16384"
                  "x-systemd.requires=modprobe@9pnet_virtio.service"
                  "access=any"
                ];
              };
              boot.kernelParams = [ "console=ttyAMA0,115200" "console=tty0" ];
            };
          })
        ];
      };
      acosGuestMacDeveloper = acosGuestMac.extendModules {
        modules = [ ({ ... }: { acos.profile = "developer"; }) ];
      };
      acosGuestQemuDeveloper = acosGuestQemu.extendModules {
        modules = [ ({ ... }: { acos.profile = "developer"; }) ];
      };
    in {
      nixosConfigurations.acos-aarch64 = acosGuestMac;
      nixosConfigurations.acos-aarch64-developer = acosGuestMacDeveloper;
      nixosConfigurations.acos-aarch64-qemu = acosGuestQemu;
      nixosConfigurations.acos-aarch64-qemu-developer = acosGuestQemuDeveloper;

      devShells = forAllSystems (system:
        let pkgs = import nixpkgs { inherit system; };
        in {
          default = pkgs.mkShell {
            packages = with pkgs; [
              rustc cargo rustfmt clippy
              just
              git
              jq
              yamllint
            ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ qemu podman ];

            shellHook = ''
              echo "ACOS development shell — ${system}"
              echo "Linux VM and OCI integration tests require a Linux host or Linux VM."
            '';
          };
        });
    };
}
