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
    ./alerts.nix
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

  services.cloud-init.enable = false;
  networking.useDHCP = false;
  services.resolved.enable = true;

  systemd.network = {
    enable = true;
    wait-online.anyInterface = true;
    networks."10-wan" = {
      matchConfig.PermanentMACAddress = "ee:e8:d4:b9:80:b7";
      address = [
        "76.13.146.117/24"
        "2a02:4780:41:66::1/48"
      ];
      routes = [
        { Gateway = "76.13.146.254"; }
        { Gateway = "2a02:4780:41::1"; }
      ];
      dns = [
        "153.92.2.6"
        "1.1.1.1"
        "8.8.4.4"
      ];
      networkConfig = {
        IPv6AcceptRA = false;
        DNSDefaultRoute = true;
      };
      linkConfig.RequiredForOnline = "routable";
    };
  };

  networking.firewall.allowedTCPPorts = [
    22
    80
    443
  ];

  system.stateVersion = "25.11";
}
