{ config, pkgs, ... }:
{
  services.restic.backups.overdueprogress = {
    initialize = true;
    paths = [ "/var/lib/overdueprogress/snapshot.db" ];
    repositoryFile = config.sops.secrets.restic_repository.path;
    passwordFile = config.sops.secrets.restic_password.path;
    environmentFile = config.sops.secrets.restic_env.path;
    backupPrepareCommand = ''
      ${pkgs.sqlite}/bin/sqlite3 \
        /var/lib/overdueprogress/submissions.db \
        ".backup /var/lib/overdueprogress/snapshot.db"
    '';
    timerConfig = {
      OnCalendar = "daily";
      Persistent = true;
    };
    pruneOpts = [
      "--keep-daily 7"
      "--keep-weekly 4"
      "--keep-monthly 6"
    ];
  };

  sops.secrets.restic_repository = { };
  sops.secrets.restic_password = { };
  sops.secrets.restic_env = { };
}
