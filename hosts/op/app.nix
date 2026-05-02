{ config, ... }:
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
}
