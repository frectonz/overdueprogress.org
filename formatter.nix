{ pkgs, ... }:
let
  prettier = pkgs.writeShellApplication {
    name = "prettier";
    runtimeInputs = [
      pkgs.nodejs
      pkgs.pnpm
    ];
    text = ''
      exec pnpm exec prettier "$@"
    '';
  };
in
pkgs.treefmt.withConfig {
  runtimeInputs = [
    pkgs.nixfmt
    prettier
  ];
  settings = {
    on-unmatched = "info";
    global.excludes = [ "pnpm-lock.yaml" ];
    formatter.nixfmt = {
      command = "nixfmt";
      includes = [ "*.nix" ];
    };
    formatter.prettier = {
      command = "prettier";
      options = [ "--write" ];
      includes = [
        "*.astro"
        "*.ts"
        "*.mjs"
        "*.json"
        "*.jsonc"
        "*.css"
        "*.md"
        "*.yml"
        ".prettierrc"
      ];
    };
  };
}
