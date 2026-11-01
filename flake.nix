{
  outputs =
    { self }:
    let
      lock = builtins.fromJSON (builtins.readFile ./nix/flake.lock);
      nixpkgsLocked = lock.nodes.nixpkgs.locked;
      nixpkgsUrl =
        if nixpkgsLocked.type == "github" then
          "https://github.com/${nixpkgsLocked.owner}/${nixpkgsLocked.repo}/archive/${nixpkgsLocked.rev}.tar.gz"
        else
          nixpkgsLocked.url;
      nixpkgs = builtins.getFlake (
        "tarball+" + nixpkgsUrl + "?narHash=" + nixpkgsLocked.narHash
      );
      flake = import ./nix/flake.nix;
    in
    flake.outputs {
      inherit self nixpkgs;
    };
}
