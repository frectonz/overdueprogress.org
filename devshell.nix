{
  pkgs,
  ...
}:
let
  rust = pkgs.rust-bin.stable.latest.default;

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
    runtimeInputs = [ pkgs.nh ];
    text = ''
      nh os switch \
        --hostname op \
        --target-host ${target} \
        --build-host ${target} \
        --elevation-strategy none \
        --diff never \
        "$@" \
        .
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
        --build-on-remote \
        --extra-files "$tmp" \
        ${target} "$@"
    '';
  };

  restic-op = pkgs.writeShellApplication {
    name = "restic-op";
    runtimeInputs = [
      pkgs.restic
      pkgs.sops
      pkgs.coreutils
    ];
    text = ''
      set -euo pipefail
      secrets=hosts/op/secrets.yaml
      if [ ! -f "$secrets" ]; then
        echo "restic-op: $secrets not found (run from repo root)" >&2
        exit 1
      fi
      RESTIC_REPOSITORY=$(sops --decrypt --extract '["restic_repository"]' "$secrets")
      RESTIC_PASSWORD=$(sops --decrypt --extract '["restic_password"]' "$secrets")
      export RESTIC_REPOSITORY RESTIC_PASSWORD
      env_file=$(mktemp)
      trap 'rm -f "$env_file"' EXIT
      chmod 600 "$env_file"
      sops --decrypt --extract '["restic_env"]' "$secrets" > "$env_file"
      set -a
      # shellcheck disable=SC1090
      . "$env_file"
      set +a
      if [ "$#" -eq 0 ]; then
        cat <<'EOF' >&2
restic-op: thin wrapper around `restic` with the op host's R2 creds loaded.

Common commands:
  restic-op snapshots                       list backups
  restic-op snapshots --json                machine-readable list
  restic-op restore latest --target ./out   restore latest snapshot to ./out
  restic-op restore <id> --target ./out     restore a specific snapshot
  restic-op dump latest /var/lib/overdueprogress/snapshot.db > restored.db
  restic-op mount /tmp/restic-mnt           browse snapshots as files
  restic-op stats                           repo stats

Pass any restic args after `restic-op`.
EOF
        exit 1
      fi
      exec restic "$@"
    '';
  };
in
pkgs.mkShellNoCC {
  packages = [
    rust
    pkgs.sqlx-cli
    pkgs.sqlite
    pkgs.pkg-config
    pkgs.openssl
    pkgs.rust-analyzer

    pkgs.nixos-rebuild
    pkgs.nixos-anywhere
    pkgs.nh
    pkgs.sops
    pkgs.ssh-to-age
    pkgs.age
    pkgs.pwgen

    server-login
    server-switch
    server-deploy
    telegram-chat-id
    restic-op
  ];

  shellHook = ''
    export DATABASE_URL="sqlite://$PWD/submissions.db"
    export SOPS_AGE_KEY_FILE="$HOME/.config/sops/age/keys.txt"
  '';
}
