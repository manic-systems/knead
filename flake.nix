{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs?ref=nixos-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    kdl = {
      url = "github:kdl-org/kdl/89c1087d5e7f530de328f18b6a0fad54ca8ea227";
      flake = false;
    };
    kdl-v1 = {
      url = "github:kdl-org/kdl/654ab5deb31e820899a41219526ffcc61ee39353";
      flake = false;
    };
  };

  outputs =
    { self, ... }@inputs:
    let
      inherit (inputs)
        nixpkgs
        fenix
        kdl
        kdl-v1
        ;
      inherit (nixpkgs) lib;
      forAllSystems = lib.genAttrs (lib.systems.doubles.linux ++ lib.systems.doubles.darwin);
      pkgsFor = system: nixpkgs.legacyPackages.${system} or (import nixpkgs { inherit system; });

      rustfmtFor = pkgs: system: fenix.packages.${system}.latest.rustfmt or pkgs.rustfmt;

      # wild + clang are only used on Linux tier-1 arches
      hasWild = plat: plat.isLinux && (plat.isx86_64 || plat.isAarch64);

      nativeDeps =
        pkgs:
        lib.optionals (hasWild pkgs.stdenv.hostPlatform) [
          pkgs.wild
          pkgs.clang
        ];
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          knead = pkgs.callPackage ./nix/package.nix {
            specCorpusV2 = "${kdl}/tests/test_cases";
            specCorpusV1 = "${kdl-v1}/tests/test_cases";
          };
        in
        {
          inherit knead;
          default = knead;
        }
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          default = pkgs.callPackage ./nix/shell.nix {
            rustfmt = rustfmtFor pkgs system;
            extraPackages = nativeDeps pkgs;
          };
        }
      );

      checks = forAllSystems (system: {
        knead = self.packages.${system}.knead;
      });
    };
}
