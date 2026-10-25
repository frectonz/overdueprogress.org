{
  description = "Overdue Progress";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs =
    { self, nixpkgs }:
    let
      forAllSystems =
        fn:
        let
          systems = [
            "x86_64-linux"
            "aarch64-darwin"
          ];
          overlays = [ ];
        in
        nixpkgs.lib.genAttrs systems (
          system:
          fn (
            import nixpkgs {
              inherit system overlays;
            }
          )
        );
    in
    {
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          buildInputs = [
            pkgs.pnpm
            pkgs.nodejs
            pkgs.emmet-ls
            pkgs.astro-language-server
            pkgs.typescript-language-server
            pkgs.vscode-langservers-extracted
          ];
        };
      });

      formatter = forAllSystems (
        pkgs:
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
      );
    };
}
