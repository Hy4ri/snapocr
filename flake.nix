{
  description = "snapocr — lean & fast screen OCR to clipboard for Wayland & X11";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f:
        nixpkgs.lib.genAttrs systems (system: f (import nixpkgs { inherit system; }));

      tesseractFull = pkgs:
        pkgs.tesseract.override { enableLanguages = [ "eng" "ara" "osd" ]; };
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
            wl-clipboard
          ];
        };
      });

      packages = forAllSystems (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "snapocr";
          version = "0.2.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = with pkgs; [ pkg-config makeWrapper ];
          buildInputs = with pkgs; [ wayland libxkbcommon libgbm libGL ];
          postInstall = ''
            wrapProgram $out/bin/snapocr \
              --prefix PATH : ${pkgs.lib.makeBinPath [ (tesseractFull pkgs) pkgs.libnotify pkgs.wl-clipboard ]} \
              --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath [ pkgs.wayland pkgs.libxkbcommon pkgs.libgbm pkgs.libGL ]}
          '';
        };
      });
    };
}
