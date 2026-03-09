{
  description = "nami-core — shared browser core: HTML parsing, CSS cascade, layout, DOM, networking";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    substrate = {
      url = "github:pleme-io/substrate";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crate2nix.url = "github:nix-community/crate2nix";
  };

  outputs =
    {
      self,
      nixpkgs,
      substrate,
      crate2nix,
      ...
    }:
    let
      system = "aarch64-darwin";
      pkgs = import nixpkgs { inherit system; };
      rustLibrary = import "${substrate}/lib/rust-library.nix" {
        inherit system nixpkgs;
        nixLib = substrate;
        inherit crate2nix;
      };
      lib = rustLibrary {
        name = "nami-core";
        src = ./.;
      };
    in
    {
      inherit (lib) packages devShells apps;

      overlays.default = final: prev: {
        nami-core = self.packages.${final.system}.default;
      };

      formatter.${system} = pkgs.nixfmt-tree;
    };
}
