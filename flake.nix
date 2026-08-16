{
  description = "snapocr — zero-dependency Wayland screen OCR to clipboard";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f:
        nixpkgs.lib.genAttrs systems (system: f (import nixpkgs { inherit system; }));

      tesseractFull = pkgs:
        pkgs.tesseract.override { enableLanguages = [ "eng" "ara" "osd" ]; };

      runtimeLibs = pkgs: with pkgs; [ wayland libxkbcommon libgbm libGL ];
    in
    {
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            rustc
            cargo
            pkg-config
            (tesseractFull pkgs)
            wayland
            libxkbcommon
            libgbm
            libGL
            libnotify
          ];
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (runtimeLibs pkgs);
        };
      });

      packages = forAllSystems (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "snapocr";
          version = "1.0.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = with pkgs; [ pkg-config makeWrapper ];
          buildInputs = with pkgs; [ wayland libxkbcommon libgbm libGL ];
          postInstall = ''
            wrapProgram $out/bin/snapocr \
              --prefix PATH : ${pkgs.lib.makeBinPath [ (tesseractFull pkgs) pkgs.libnotify ]} \
              --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath (runtimeLibs pkgs)}
          '';
        };
      });
    };
}
