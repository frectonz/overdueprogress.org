{
  config,
  pkgs,
  ...
}:
let
  notifier = "${config.services.overdueprogress.package}/bin/overdueprogress-notify";
  envFile = config.sops.secrets.telegram_alert_env.path;
  onFailure = [ "telegram-notify@%n.service" ];

  oneshot = desc: args: {
    description = desc;
    path = [ pkgs.systemd ];
    serviceConfig = {
      Type = "oneshot";
      EnvironmentFile = envFile;
      ExecStart = "${notifier} ${args}";
    };
  };

  sshLoginHook = pkgs.writeShellScript "telegram-ssh-login-hook" ''
    [ "''${PAM_TYPE:-}" = "open_session" ] || exit 0
    (
      set -a
      . ${envFile}
      set +a
      exec ${notifier} ssh-login
    ) </dev/null >/dev/null 2>&1 &
    disown 2>/dev/null || true
    exit 0
  '';
in
{
  sops.secrets.telegram_alert_env = { };

  systemd.services."telegram-notify@" = oneshot "Telegram alert for %i" "failure %i";

  systemd.services.boot-notify = (oneshot "Telegram boot notification" "boot") // {
    after = [ "network-online.target" ];
    wants = [ "network-online.target" ];
    wantedBy = [ "multi-user.target" ];
  };

  systemd.services.failed-units-check =
    oneshot "Scan for systemd units in failed state" "failed-units";

  systemd.timers.failed-units-check = {
    description = "Periodic failed-units scan";
    wantedBy = [ "timers.target" ];
    timerConfig = {
      OnBootSec = "10min";
      OnUnitActiveSec = "15min";
      Unit = "failed-units-check.service";
    };
  };

  systemd.services.disk-check =
    let
      base = oneshot "Disk usage threshold check" "disk-check";
    in
    base
    // {
      serviceConfig = base.serviceConfig // {
        StateDirectory = "telegram-notify";
      };
    };

  systemd.timers.disk-check = {
    description = "Periodic disk usage check";
    wantedBy = [ "timers.target" ];
    timerConfig = {
      OnBootSec = "5min";
      OnUnitActiveSec = "30min";
      Unit = "disk-check.service";
    };
  };

  security.pam.services.sshd.rules.session.telegram-ssh-login = {
    control = "optional";
    modulePath = "${pkgs.pam}/lib/security/pam_exec.so";
    args = [
      "quiet"
      (toString sshLoginHook)
    ];
    order = config.security.pam.services.sshd.rules.session.unix.order + 10000;
  };

  systemd.services.overdueprogress.unitConfig.OnFailure = onFailure;
  systemd.services.caddy.unitConfig.OnFailure = onFailure;
  systemd.services.restic-backups-overdueprogress.unitConfig.OnFailure = onFailure;
}
