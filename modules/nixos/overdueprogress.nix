{
  lib,
  config,
  pkgs,
  flake,
  ...
}:
let
  cfg = config.services.overdueprogress;
in
{
  options.services.overdueprogress = {
    enable = lib.mkEnableOption "Overdue Progress submission server";

    package = lib.mkOption {
      type = lib.types.package;
      default = flake.packages.${pkgs.system}.overdueprogress;
    };

    environmentFile = lib.mkOption {
      type = lib.types.path;
    };

    address = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1:3000";
    };

    user = lib.mkOption {
      type = lib.types.str;
      default = "overdueprogress";
    };

    group = lib.mkOption {
      type = lib.types.str;
      default = "overdueprogress";
    };
  };

  config = lib.mkIf cfg.enable {
    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.group;
      home = "/var/lib/overdueprogress";
    };
    users.groups.${cfg.group} = { };

    systemd.services.overdueprogress = {
      description = "Overdue Progress submission server";
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
      wantedBy = [ "multi-user.target" ];

      serviceConfig = {
        Type = "simple";
        User = cfg.user;
        Group = cfg.group;
        ExecStart = lib.getExe cfg.package;
        EnvironmentFile = cfg.environmentFile;
        Environment = [
          "ADDR=${cfg.address}"
          "DATABASE_URL=sqlite:///var/lib/overdueprogress/submissions.db"
        ];

        StateDirectory = "overdueprogress";
        StateDirectoryMode = "0700";
        WorkingDirectory = "/var/lib/overdueprogress";
        ReadWritePaths = [ "/var/lib/overdueprogress" ];

        Restart = "on-failure";
        RestartSec = 5;

        NoNewPrivileges = true;
        PrivateTmp = true;
        PrivateDevices = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectControlGroups = true;
        RestrictRealtime = true;
        RestrictSUIDSGID = true;
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        SystemCallArchitectures = "native";
      };
    };
  };
}
