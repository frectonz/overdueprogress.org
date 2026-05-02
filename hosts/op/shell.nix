{ pkgs, ... }:
{
  users.users.root.shell = pkgs.fish;

  programs.fish = {
    enable = true;
    shellAliases = {
      ls = "lsd --group-directories-first -al";
      cat = "bat";
      df = "duf";
    };
    shellAbbrs = {
      lg = "lazygit";
      stat = "git status";
      logs = "journalctl -u overdueprogress -f";
      tail-caddy = "journalctl -u caddy -f";
    };
  };

  programs.starship.enable = true;
}
