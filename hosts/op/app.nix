{
  config,
  pkgs,
  ...
}:
{
  services.overdueprogress = {
    enable = true;
    environmentFile = config.sops.secrets.overdueprogress_env.path;
    address = "127.0.0.1:3000";
  };

  sops.secrets.overdueprogress_env = {
    owner = config.services.overdueprogress.user;
    mode = "0400";
  };

  services.caddy = {
    enable = true;
    email = "fraol0912@gmail.com";
    virtualHosts."overdueprogress.org".extraConfig = ''
      reverse_proxy 127.0.0.1:3000
    '';
    virtualHosts."www.overdueprogress.org".extraConfig = ''
      redir https://overdueprogress.org{uri} permanent
    '';
  };

  services.restic.backups.overdueprogress = {
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
