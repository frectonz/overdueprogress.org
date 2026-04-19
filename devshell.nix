{
  pkgs,
  inputs,
  ...
}:
let
  pkgs' = pkgs.appendOverlays [ (import inputs.rust-overlay) ];
  rust = pkgs'.rust-bin.stable.latest.default;

  vars = import ./hosts/op/variables.nix;
  target = "root@${vars.sshHost}";

  server-login = pkgs.writeShellApplication {
    name = "server-login";
    text = ''
      ssh ${target}
    '';
  };

  server-switch = pkgs.writeShellApplication {
    name = "server-switch";
    runtimeInputs = [ pkgs.nixos-rebuild ];
    text = ''
      nixos-rebuild \
        --flake .#op \
        --target-host ${target} \
        --build-host ${target} \
        switch "$@"
    '';
  };

  telegram-chat-id = pkgs.writeShellApplication {
    name = "telegram-chat-id";
    runtimeInputs = [
      pkgs.curl
      pkgs.jq
    ];
    text = ''
      set -euo pipefail
      if [ ! -f .env ]; then
        echo "no .env file found" >&2
        exit 1
      fi
      token=$(grep -E '^TELEGRAM_BOT_TOKEN=' .env | cut -d= -f2- | tr -d '"' | tr -d "'")
      if [ -z "$token" ]; then
        echo "TELEGRAM_BOT_TOKEN not set in .env" >&2
        exit 1
      fi
      curl -s "https://api.telegram.org/bot$token/getUpdates" \
        | jq -r '.result[].message.chat | "\(.id)\t\(.username // .first_name)"' \
        | sort -u
    '';
  };

  server-deploy = pkgs.writeShellApplication {
    name = "server-deploy";
    runtimeInputs = [
      pkgs.nixos-anywhere
      pkgs.sops
      pkgs.coreutils
    ];
    text = ''
      set -euo pipefail
      tmp=$(mktemp -d)
      trap 'rm -rf "$tmp"' EXIT
      install -d -m 0755 "$tmp/etc/ssh"
      sops --decrypt --extract '["ssh_host_ed25519_key"]' \
        hosts/op/secrets.yaml > "$tmp/etc/ssh/ssh_host_ed25519_key"
      chmod 600 "$tmp/etc/ssh/ssh_host_ed25519_key"
      nixos-anywhere \
        --flake .#op \
        --extra-files "$tmp" \
        ${target} "$@"
    '';
  };
in
pkgs'.mkShellNoCC {
  packages = [
    rust
    pkgs.sqlx-cli
    pkgs.sqlite
    pkgs.pkg-config
    pkgs.openssl
    pkgs.rust-analyzer

    pkgs.nixos-rebuild
    pkgs.nixos-anywhere
    pkgs.sops
    pkgs.ssh-to-age
    pkgs.age
    pkgs.pwgen

    server-login
    server-switch
    server-deploy
    telegram-chat-id
  ];

  shellHook = ''
    export DATABASE_URL="sqlite://$PWD/submissions.db"
    export SOPS_AGE_KEY_FILE="$HOME/.config/sops/age/keys.txt"
  '';
}
