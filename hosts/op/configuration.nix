{
  flake,
  inputs,
  modulesPath,
  ...
}:
{
  imports = [
    flake.nixosModules.server
    flake.nixosModules.overdueprogress
    ./disko.nix
    ./app.nix
    ./shell.nix
    (modulesPath + "/profiles/qemu-guest.nix")
  ];

  nixpkgs.hostPlatform = "x86_64-linux";

  networking.hostName = "op";

  boot.loader.grub = {
    enable = true;
    device = "nodev";
    efiSupport = true;
    efiInstallAsRemovable = true;
  };

  boot.initrd.availableKernelModules = [
    "ahci"
    "virtio_pci"
    "virtio_blk"
    "virtio_scsi"
    "virtio_net"
    "xhci_pci"
    "sd_mod"
    "sr_mod"
  ];

  time.timeZone = "Africa/Addis_Ababa";
  i18n.defaultLocale = "en_US.UTF-8";

  sops.defaultSopsFile = ./secrets.yaml;

  networking.firewall.allowedTCPPorts = [
    22
    80
    443
  ];

  system.stateVersion = "25.11";
}
