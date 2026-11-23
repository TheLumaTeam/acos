{ config, lib, pkgs, ... }:

let
  cfg = config.acos.desktop;
  desktopLauncher = pkgs.writeShellScriptBin "acos-desktop" ''
    export ACOS_CHROMIUM=${lib.getExe pkgs.chromium}
    export ACOS_AGENT_NAME=${lib.escapeShellArg cfg.agentName}
    exec ${pkgs.python3}/bin/python3 ${../desktop/desktop.py}
  '';
in
{
  options.acos.profile = lib.mkOption {
    type = lib.types.enum [ "runtime" "developer" ];
    default = "runtime";
    description = "Guest package profile. The runtime profile is optimized for a small downloadable image.";
  };

  options.acos.desktop = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Enable the ACOS graphical desktop and application launcher.";
    };

    agentName = lib.mkOption {
      type = lib.types.str;
      default = "ACOS Agent";
      description = "Agent identity shown in the desktop shell.";
    };

    agentLogo = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = "Optional agent logo image supplied by the image builder.";
    };
  };

  config = lib.mkIf cfg.enable {
    # Install only the compositor binary, not NixOS's general-purpose
    # Wayland-session module (which pulls portals, GTK, Xwayland, audio and
    # desktop integration services into the image).
    environment.systemPackages = [ pkgs.labwc pkgs.foot pkgs.wlr-randr desktopLauncher ];
    fonts.packages = [ pkgs.dejavu_fonts ];
    environment.variables.XDG_CURRENT_DESKTOP = "ACOS";
    hardware.graphics.enable = true;

    services.graphical-desktop.enable = lib.mkForce false;
    # systemd-logind needs the system bus to create the Wayland seat/session.
    services.dbus.enable = true;
    services.pipewire.enable = lib.mkForce false;
    services.speechd.enable = lib.mkForce false;
    xdg.portal.enable = lib.mkForce false;
    security.polkit.enable = lib.mkForce false;
    programs.dconf.enable = lib.mkForce false;
    programs.xwayland.enable = lib.mkForce false;

    environment.etc = {
      "acos/desktop/index.html".source = ../desktop/index.html;
      "xdg/labwc/autostart".source = ../desktop/labwc-autostart;
      "xdg/foot/foot.ini".text = ''
        [main]
        font=DejaVu Sans Mono:size=11
      '';
    } // lib.optionalAttrs (cfg.agentLogo != null) {
      "acos/desktop/agent-logo".source = cfg.agentLogo;
    };

    programs.bash.loginShellInit = ''
      umask 0002
      if [ "''${USER:-}" = "agent" ] && [ "$(tty 2>/dev/null)" = "/dev/tty1" ] && [ -z "''${WAYLAND_DISPLAY:-}" ]; then
        export XDG_SESSION_TYPE=wayland
        exec ${pkgs.dbus}/bin/dbus-run-session ${lib.getExe pkgs.labwc}
      fi
    '';
  };
}
