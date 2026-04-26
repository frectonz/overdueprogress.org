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
      header {
        Strict-Transport-Security "max-age=31536000; includeSubDomains"
        X-Content-Type-Options "nosniff"
        X-Frame-Options "DENY"
        Referrer-Policy "strict-origin-when-cross-origin"
        Permissions-Policy "camera=(), microphone=(), geolocation=(), interest-cohort=()"
        Content-Security-Policy "default-src 'self'; script-src 'self' 'unsafe-inline' https://challenges.cloudflare.com https://static.cloudflareinsights.com; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; font-src 'self' https://fonts.gstatic.com; img-src 'self' data:; connect-src 'self' https://cloudflareinsights.com; frame-src https://challenges.cloudflare.com; frame-ancestors 'none'; base-uri 'self'; form-action 'self'"
      }
      reverse_proxy 127.0.0.1:3000
    '';
    virtualHosts."www.overdueprogress.org".extraConfig = ''
      redir https://overdueprogress.org{uri} permanent
    '';
  };

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
