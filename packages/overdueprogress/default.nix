{
  pkgs,
  flake,
  inputs,
  ...
}:
let
  pkgs' = pkgs.appendOverlays [ (import inputs.rust-overlay) ];
  rustToolchain = pkgs'.rust-bin.stable.latest.default;
  craneLib = (inputs.crane.mkLib pkgs').overrideToolchain rustToolchain;

  manifest = (fromTOML (builtins.readFile (flake + "/Cargo.toml"))).package;

  fullSrc = pkgs.lib.cleanSourceWith {
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
      || rel == "views"
      || rel == "static"
      || rel == ".sqlx"
      || pkgs.lib.hasPrefix "src/" rel
      || pkgs.lib.hasPrefix "migrations/" rel
      || pkgs.lib.hasPrefix "views/" rel
      || pkgs.lib.hasPrefix "static/" rel
      || pkgs.lib.hasPrefix ".sqlx/" rel;
  };

  commonArgs = {
    strictDeps = true;
    nativeBuildInputs = [ pkgs.pkg-config ];
    buildInputs = [ pkgs.openssl ];
    env.SQLX_OFFLINE = "true";
  };

  cargoArtifacts = craneLib.buildDepsOnly (
    commonArgs
    // {
      pname = "${manifest.name}-deps";
      version = manifest.version;
      src = craneLib.cleanCargoSource flake;
    }
  );
in
craneLib.buildPackage (
  commonArgs
  // {
    inherit cargoArtifacts;
    pname = manifest.name;
    version = manifest.version;
    src = fullSrc;
    doCheck = false;

    meta = {
      description = "Overdue Progress submission server";
      homepage = "https://overdueprogress.org";
      license = pkgs.lib.licenses.mit;
      mainProgram = "overdueprogress";
    };
  }
)
