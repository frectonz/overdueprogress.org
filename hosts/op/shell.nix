{ flake, pkgs, ... }:
{
  imports = [ flake.inputs.nix-index-database.nixosModules.nix-index ];

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

  programs.nh = {
    enable = true;
    clean.enable = true;
    clean.extraArgs = "--keep-since 4d --keep 3";
  };

  programs.nix-index-database.comma.enable = true;

  environment.systemPackages = [
    pkgs.bat
    pkgs.btop
    pkgs.duf
    pkgs.git
    pkgs.helix
    pkgs.lazygit
    pkgs.lsd
    pkgs.ripgrep
    pkgs.ghostty.terminfo
    pkgs.kitty.terminfo
  ];
}
