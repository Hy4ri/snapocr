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
            grim
            slurp
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
          postInstall = ''
            wrapProgram $out/bin/snapocr \
              --prefix PATH : ${pkgs.lib.makeBinPath [ (tesseractFull pkgs) pkgs.grim pkgs.slurp pkgs.libnotify pkgs.wl-clipboard ]}
          '';
        };
      });
    };
}
