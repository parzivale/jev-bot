{
  description = "Discord bot that scores statement truthiness with the TypeSafe jev model";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAll (pkgs: rec {
        jev-bot = pkgs.callPackage ./package.nix { };
        default = jev-bot;
      });

      apps = forAll (pkgs:
        let
          program = nixpkgs.lib.getExe self.packages.${pkgs.stdenv.hostPlatform.system}.jev-bot;
        in
        rec {
          jev-bot = { type = "app"; inherit program; };
          default = jev-bot;
        });

      overlays.default = final: _prev: {
        jev-bot = final.callPackage ./package.nix { };
      };

      nixosModules.jev-bot = import ./module.nix { inherit self; };
      nixosModules.default = self.nixosModules.jev-bot;

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [ cargo rustc rustfmt clippy rust-analyzer pkg-config ];
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
          shellHook = ''
            echo "jev-bot dev shell — $(rustc --version)"
          '';
        };
      });

      formatter = forAll (pkgs: pkgs.nixpkgs-fmt);
    };
}
