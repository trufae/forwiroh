{
  lib,
  rustPlatform,
}:

rustPlatform.buildRustPackage {
  pname = "forwiroh";
  version = (lib.importTOML ../Cargo.toml).workspace.package.version;

  src = lib.cleanSource ../.;

  cargoDeps = rustPlatform.importCargoLock {
    lockFile = ../Cargo.lock;
  };

  cargoBuildFlags = [
    "--package"
    "forwiroh-cli"
  ];

  cargoInstallFlags = [
    "--path"
    "crates/forwiroh-cli"
  ];

  meta = {
    description = "Secure paired-device networking over Iroh";
    homepage = "https://github.com/trufae/forwiroh";
    license = lib.licenses.mit;
    mainProgram = "forwiroh";
    platforms = lib.platforms.unix;
  };
}
