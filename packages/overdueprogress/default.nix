{
  pkgs,
  flake,
  ...
}:
let
  manifest = (fromTOML (builtins.readFile (flake + "/Cargo.toml"))).package;

  src = pkgs.lib.cleanSourceWith {
    src = flake;
    filter =
      path: _:
      let
        rel = pkgs.lib.removePrefix (toString flake + "/") (toString path);
      in
      rel == ""
      || rel == "Cargo.toml"
      || rel == "Cargo.lock"
      || rel == "RUST_LOG.txt"
      || rel == "src"
      || rel == "migrations"
      || rel == "templates"
      || rel == "static"
      || rel == ".sqlx"
      || pkgs.lib.hasPrefix "src/" rel
      || pkgs.lib.hasPrefix "migrations/" rel
      || pkgs.lib.hasPrefix "templates/" rel
      || pkgs.lib.hasPrefix "static/" rel
      || pkgs.lib.hasPrefix ".sqlx/" rel;
  };
in
pkgs.rustPlatform.buildRustPackage {
  pname = manifest.name;
  version = manifest.version;
  inherit src;

  cargoLock.lockFile = flake + "/Cargo.lock";

  nativeBuildInputs = [ pkgs.pkg-config ];
  buildInputs = [ pkgs.openssl ];

  env.SQLX_OFFLINE = "true";

  doCheck = false;

  meta = {
    description = "Overdue Progress submission server";
    homepage = "https://overdueprogress.org";
    license = pkgs.lib.licenses.mit;
    mainProgram = "overdueprogress";
  };
}
