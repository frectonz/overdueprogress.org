{
  flake,
  modulesPath,
  ...
}:
{
  imports = [
    flake.nixosModules.server
    flake.nixosModules.overdueprogress
    ./alerts.nix
    ./app.nix
    ./backups.nix
    ./boot.nix
    ./caddy.nix
    ./disko.nix
    ./locale.nix
    ./networking.nix
    ./nh.nix
    ./nix-index.nix
    ./nixpkgs.nix
    ./packages.nix
    ./shell.nix
    ./sops.nix
    (modulesPath + "/profiles/qemu-guest.nix")
  ];

  system.stateVersion = "25.11";
  system.configurationRevision = flake.rev or flake.dirtyRev or null;
}
