{
  config,
  lib,
  pkgs,
  primaryUser ? null,
  ...
}:

let
  cfg = config.programs.forwiroh;

  inherit (lib.options) mkEnableOption mkOption;
  inherit (lib.modules) mkIf;
  inherit (lib.types) package str;

  defaultUser = if primaryUser == null then "forwiroh" else primaryUser;
  defaultStateDirectory =
    if primaryUser == null then
      "/var/lib/forwiroh"
    else
      "${config.users.users.${primaryUser}.home}/.local/state/forwiroh";
  userGroup = config.users.users.${cfg.user}.group;
  defaultPackage = pkgs.callPackage ./package.nix { };
in
{
  options.programs.forwiroh = {
    enable = mkEnableOption "Forwiroh peer networking daemon";

    package = mkOption {
      type = package;
      default = defaultPackage;
      description = "Forwiroh package to install and run.";
    };

    user = mkOption {
      type = str;
      default = defaultUser;
      description = "User that owns the Forwiroh identity and daemon.";
    };

    name = mkOption {
      type = str;
      default = "computer";
      description = "Name announced by the Forwiroh daemon.";
    };

    stateDirectory = mkOption {
      type = str;
      default = defaultStateDirectory;
      description = "Directory containing the Forwiroh identity and peer state.";
    };
  };

  config = mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];

    users.groups.forwiroh = mkIf (cfg.user == "forwiroh" && primaryUser == null) { };
    users.users.forwiroh = mkIf (cfg.user == "forwiroh" && primaryUser == null) {
      isSystemUser = true;
      group = "forwiroh";
      home = "/var/lib/forwiroh";
      createHome = true;
    };

    systemd.tmpfiles.rules = [
      "d ${cfg.stateDirectory} 0700 ${cfg.user} ${userGroup} -"
    ];

    systemd.services.forwiroh = {
      description = "Forwiroh peer networking daemon";
      wants = [ "network-online.target" ];
      after = [ "network-online.target" ];
      wantedBy = [ "multi-user.target" ];

      serviceConfig = {
        ExecStart = "${cfg.package}/bin/forwiroh --state ${cfg.stateDirectory} serve --name ${cfg.name}";
        User = cfg.user;
        Group = userGroup;
        Restart = "on-failure";
        RestartSec = "5s";
        UMask = "0077";
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ReadWritePaths = [ cfg.stateDirectory ];
      };
    };
  };
}
