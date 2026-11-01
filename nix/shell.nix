{
  pkgs ? import <nixpkgs> { },
}:

pkgs.mkShell {
  strictDeps = true;

  packages = with pkgs; [
    bashInteractive
    cargo
    clippy
    git
    iproute2
    rustc
    rustfmt
  ];

  nativeBuildInputs = [ pkgs.pkg-config ];
}
