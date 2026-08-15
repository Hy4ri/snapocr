{
  description = "snapocr — select a screen region, OCR it, copy to clipboard";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f:
        nixpkgs.lib.genAttrs systems (system: f (import nixpkgs { inherit system; }));

      tesseractFull = pkgs:
        pkgs.tesseract.override { enableLanguages = [ "eng" "ara" ]; };

      runtimeLibs = pkgs: with pkgs; [ libGL libxkbcommon wayland xorg.libxcb ];
    in
    {
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            rustc
            cargo
            pkg-config
            (tesseractFull pkgs)
            # x11/wayland libs for eframe + xcap
            xorg.libxcb
            xorg.xcbproto
            libxkbcommon
            wayland
            libGL
            fontconfig
            dejavu_fonts
            # xcap wayland capture backend (pipewire portal)
            pipewire
            libgbm
            # pipewire-sys bindgen needs libclang
            clang
            libclang.lib
            # headless test rig
            xvfb-run
            xdotool
            feh
            xclip
            imagemagick
          ];
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (runtimeLibs pkgs);
          LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
        };
      });

      packages = forAllSystems (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "snapocr";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = with pkgs; [ pkg-config makeWrapper ];
          buildInputs = with pkgs; [ xorg.libxcb xorg.xcbproto libxkbcommon wayland libGL pipewire ];
          postInstall = ''
            wrapProgram $out/bin/snapocr \
              --prefix PATH : ${pkgs.lib.makeBinPath [ (tesseractFull pkgs) ]} \
              --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath (runtimeLibs pkgs)}
          '';
        };
      });
    };
}
