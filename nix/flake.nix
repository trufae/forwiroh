{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    {
      self,
      nixpkgs,
    }:
    let
      inherit (nixpkgs) lib;

      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      forAllSystems = f: lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      overlays.default = final: _: {
        forwiroh = final.callPackage ./package.nix { };
      };

      nixosModules = {
        forwiroh = import ./module.nix;
        default = self.nixosModules.forwiroh;
      };

      packages = forAllSystems (pkgs: {
        forwiroh = pkgs.callPackage ./package.nix { };
        default = self.packages.${pkgs.stdenv.hostPlatform.system}.forwiroh;
      });

      apps = forAllSystems (pkgs: {
        forwiroh = {
          type = "app";
          program = "${self.packages.${pkgs.stdenv.hostPlatform.system}.forwiroh}/bin/forwiroh";
          meta = {
            description = "Secure paired-device networking over Iroh";
            mainProgram = "forwiroh";
          };
        };
        default = self.apps.${pkgs.stdenv.hostPlatform.system}.forwiroh;
      });

      devShells = forAllSystems (pkgs: {
        default = import ./shell.nix { inherit pkgs; };
      });

      checks = forAllSystems (pkgs: {
        forwiroh = self.packages.${pkgs.stdenv.hostPlatform.system}.forwiroh;
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt);
    };
}
