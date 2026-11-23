{
  description = "Reproducible ACOS ARM64 disk images";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/04906337b6d53e8bb0eaf0470ae9f070a4d4eb55";

  outputs = { self, nixpkgs }:
    let
      system = "aarch64-linux";
      pkgs = import nixpkgs { inherit system; };
      lib = nixpkgs.lib;

      profiles = {
        runtime = import ./profiles/runtime.nix;
        developer = import ./profiles/developer.nix;
      };

      mkImage = profile:
        let
          branding = profiles.${profile};
          configuration = lib.nixosSystem {
            inherit system;
            modules = [
              (nixpkgs + "/nixos/modules/virtualisation/disk-image.nix")
              ./guest/nixos/configuration.nix
              ({ ... }: {
                options.virtualisation.cores = lib.mkOption {
                  type = lib.types.ints.positive;
                  default = 2;
                  description = "CPU count recorded for ACOS image consumers.";
                };
                options.virtualisation.memorySize = lib.mkOption {
                  type = lib.types.ints.positive;
                  default = 2048;
                  description = "Guest memory size in MiB, retained as image metadata.";
                };
                config = {
                  acos.profile = profile;
                  acos.desktop.agentName = branding.agentName;
                  acos.desktop.agentLogo = ./branding/acos-mark.svg;
                  virtualisation.diskSize = 16384;
                  image.format = "qcow2";
                  image.efiSupport = true;
                  image.baseName = "acos-${profile}-aarch64";
                };
              })
            ];
          };
          diskImage = configuration.config.system.build.image;
          metadata = pkgs.writeText "acos-image-metadata.json" (builtins.toJSON {
            schema_version = 1;
            distribution = "ACOS";
            inherit profile system;
            format = "qcow2";
            architecture = "aarch64";
            nixpkgs_revision = "04906337b6d53e8bb0eaf0470ae9f070a4d4eb55";
            image = "acos-${profile}-aarch64.qcow2";
            disk_size_mib = 16384;
            inherit (branding) agentName;
          });
        in
        pkgs.runCommand "acos-${profile}-aarch64-image" {
          nativeBuildInputs = [ pkgs.coreutils pkgs.qemu ];
          passthru = {
            inherit configuration diskImage profile;
          };
        } ''
          mkdir -p "$out"
          cp "${diskImage}/${configuration.config.image.fileName}" "$out/acos-${profile}-aarch64.qcow2"
          cp ${metadata} "$out/metadata.json"
          ${pkgs.qemu}/bin/qemu-img info --output=json "$out/acos-${profile}-aarch64.qcow2" > "$out/qemu-img-info.json"
          cd "$out"
          ${pkgs.coreutils}/bin/sha256sum "acos-${profile}-aarch64.qcow2" > SHA256SUMS
        '';
    in {
      packages.${system} = {
        runtime = mkImage "runtime";
        developer = mkImage "developer";
        default = self.packages.${system}.runtime;
      };

      nixosConfigurations = {
        runtime = (mkImage "runtime").passthru.configuration;
        developer = (mkImage "developer").passthru.configuration;
      };
    };
}
