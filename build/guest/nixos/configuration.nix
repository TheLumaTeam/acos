{ config, lib, pkgs, ... }:

let
  runtimePackages = with pkgs; [
    bashInteractive coreutils findutils util-linux which file less vim nano tmux
    ripgrep fd jq git openssh rsync
    python3 uv chromium sqlite
    zip unzip xz zstd gzip gnutar curl wget iproute2 iputils dnsutils
    netcat-openbsd socat openssl cacert
  ];

  developerPackages = with pkgs; [
    zsh fish tree eza bat fzf yq-go htop btop procs ncdu dust hyperfine
    watchexec entr direnv just git-lfs gh ruff python3Packages.pytest
    nodejs_22 pnpm typescript
    python3Packages.ipython perl ruby rustc cargo rustfmt clippy shellcheck shfmt
    buildah skopeo podman-compose slirp4netns fuse-overlayfs crun passt shadow
    postgresql redis p7zip bind traceroute mtr nmap tcpdump iperf3
  ];

  agentInfo = {
    schema_version = 1;
    distribution = "ACOS";
    profile = config.acos.profile;
    agent = config.acos.desktop.agentName;
    user = "agent";
    workspace = "/workspace";
    access = {
      console = "serial-autologin";
      privilege_escalation = "passwordless-sudo-inside-guest";
    };
    capabilities = {
      shells = [ "bash" ] ++ lib.optionals (config.acos.profile == "developer") [ "zsh" "fish" ];
      languages = [ "python" "shell" ] ++ lib.optionals (config.acos.profile == "developer") [ "javascript" "typescript" "perl" "ruby" "rust" ];
      package_managers = [ "nix" "uv" "pip" ] ++ lib.optionals (config.acos.profile == "developer") [ "npm" "pnpm" "cpan" "gem" "cargo" ];
      browser = "chromium";
    } // lib.optionalAttrs (config.acos.profile == "developer") {
      containers = [ "podman" "buildah" "skopeo" ];
    } // lib.optionalAttrs config.acos.desktop.enable {
      desktop = {
        environment = "ACOS";
        compositor = "labwc";
        shell = "acos-desktop";
      };
    };
  };

  agentInfoFile = pkgs.writeText "acos-agent.json" (builtins.toJSON agentInfo);

  acosInfo = pkgs.writeShellScriptBin "acos-info" ''
    if [ "''${1:-}" = "--json" ]; then
      cat /etc/acos/agent.json
      exit 0
    fi

    printf 'ACOS agent computer\n'
    printf 'Profile: %s\n' '${agentInfo.profile}'
    printf 'User: %s\n' "$USER"
    printf 'Workspace: %s\n' "$ACOS_WORKSPACE"
    printf 'Kernel: '
    ${pkgs.coreutils}/bin/uname -sr
    printf '\nToolchains:\n'
    for tool in python3 perl ruby node rustc cargo podman git; do
      if command -v "$tool" >/dev/null 2>&1; then
        printf '  %-10s %s\n' "$tool" "$(command -v "$tool")"
      fi
    done
    printf '\nRun `acos-info --json` for the machine-readable capability manifest.\n'
  '';
in
{
  imports = [ ./desktop.nix ];

  nixpkgs.hostPlatform = lib.mkDefault "aarch64-linux";
  system.stateVersion = "26.11";

  networking = {
    hostName = "acos";
    useDHCP = true;
    firewall.enable = true;
  };

  # QEMU Guest Agent runs inside the guest and is reachable only through the
  # host-created virtio-serial channel; it does not expose a network listener.
  services.qemuGuest.enable = true;

  boot = {
    loader.systemd-boot.enable = true;
    loader.efi.canTouchEfiVariables = true;
    initrd.compressor = "zstd";
    kernelParams = [ "systemd.show_status=true" ];
    kernel.sysctl."net.ipv4.ip_forward" = 1;
  };

  time.timeZone = "UTC";
  i18n.defaultLocale = "C.UTF-8";

  users.mutableUsers = false;
  users.allowNoPasswordLogin = true;
  users.users.root.hashedPassword = "!";

  # Kept separate for a future privileged ACOS guest-side service.
  users.groups.acos = { };
  users.users.acos = {
    isSystemUser = true;
    group = "acos";
    home = "/var/lib/acos";
    createHome = true;
  };

  # This is the interactive identity used by both people and agent harnesses.
  # Its sudo access is deliberately scoped to the disposable guest, never the host.
  users.users.agent = {
    isNormalUser = true;
    uid = 1000;
    group = "users";
    home = "/home/agent";
    createHome = true;
    shell = pkgs.bashInteractive;
    extraGroups = [ "wheel" ];
    hashedPassword = "!";
    subUidRanges = [ { startUid = 100000; count = 65536; } ];
    subGidRanges = [ { startGid = 100000; count = 65536; } ];
  };

  security.sudo.wheelNeedsPassword = false;

  services.getty.autologinUser = lib.mkForce "agent";

  nix.settings = {
    experimental-features = [ "nix-command" "flakes" ];
    max-jobs = 2;
    cores = 2;
    sandbox = true;
  };

  environment.systemPackages = runtimePackages
    ++ lib.optionals (config.acos.profile == "developer") developerPackages
    ++ [ acosInfo ];
  environment.defaultPackages = lib.mkForce [ ];

  # The runtime is not expected to rebuild NixOS from inside itself. Keep
  # installer/rebuild tooling available in the developer profile only.
  system.disableInstallerTools = config.acos.profile == "runtime";

  environment.etc."acos/agent.json".source = agentInfoFile;
  environment.variables = {
    ACOS = "1";
    ACOS_PROFILE = agentInfo.profile;
    ACOS_WORKSPACE = agentInfo.workspace;
    ACOS_STATE_DIR = "/home/agent/.local/state/acos";
    ACOS_CACHE_DIR = "/home/agent/.cache";
    WORKSPACE = agentInfo.workspace;
    EDITOR = "vim";
    VISUAL = "vim";
    PAGER = "less";
  };

  programs.bash.interactiveShellInit = ''
    umask 0002
    if [ "''${USER:-}" = "agent" ] && [ -d "$ACOS_WORKSPACE" ]; then
      cd "$ACOS_WORKSPACE"
      workspace_marker="$ACOS_STATE_DIR/workspace-ownership-v1"
      if ${pkgs.util-linux}/bin/mountpoint -q /tmp/shared; then
        boot_id="$(${pkgs.coreutils}/bin/cat /proc/sys/kernel/random/boot_id)"
        if [ "$(cat "$workspace_marker" 2>/dev/null || true)" != "$boot_id" ]; then
          if sudo ${pkgs.coreutils}/bin/chown -R agent:users /tmp/shared; then
            printf '%s\n' "$boot_id" > "$workspace_marker"
          fi
        fi
      fi
    fi
  '';

  virtualisation = {
    memorySize = lib.mkDefault 2048;
    cores = lib.mkDefault 2;
    podman = {
      enable = config.acos.profile == "developer";
      defaultNetwork.settings.dns_enabled = true;
    };
  };

  # The VM launcher supplies the persistent workspace at /tmp/shared.
  # Create its alias through the existing tmpfiles setup, without another unit.
  systemd.tmpfiles.rules = [
    "d /var/lib/acos 0750 acos acos - -"
    "d /home/agent/.cache 0750 agent users - -"
    "d /home/agent/.local 0750 agent users - -"
    "d /home/agent/.local/state 0750 agent users - -"
    "d /home/agent/.local/state/acos 0750 agent users - -"
    "L+ /workspace - - - - /tmp/shared"
  ];

  # Keep the guest's boot path small.
  documentation.enable = false;
  documentation.nixos.enable = false;
  documentation.man.enable = false;
  documentation.info.enable = false;
  programs.command-not-found.enable = false;
  systemd.services.linger-users.enable = false;
  systemd.services.systemd-udev-settle.enable = false;
  services.journald.settings.Journal.Storage = "volatile";
}
