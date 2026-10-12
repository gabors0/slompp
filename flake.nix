{
  description = "SLOPP: an XAMPP-style GPUI control panel for Apache, PHP, MariaDB and phpMyAdmin";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" ] (
      system:
      let
        pkgs = import nixpkgs { inherit system; };

        # Libraries GPUI loads at runtime (wgpu/Vulkan, Wayland, X11, fonts).
        runtimeLibs = with pkgs; [
          vulkan-loader
          libGL
          wayland
          libxkbcommon
          libx11
          libxcb
          libxcursor
          libxi
          libxrandr
          fontconfig
          freetype
        ];

        # The XAMPP modules themselves. The panel finds them on PATH.
        modules = with pkgs; [
          apacheHttpd
          php
          mariadb
        ];

        buildInputs =
          runtimeLibs
          ++ (with pkgs; [
            zstd
            expat
          ]);

        nativeBuildInputs = with pkgs; [
          pkg-config
          cmake
          python3
        ];

        slopp = pkgs.rustPlatform.buildRustPackage {
          pname = "slopp";
          version = "0.1.0";
          src = pkgs.lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;
          # Unit tests run in the dev shell; the crate graph is large.
          doCheck = false;

          nativeBuildInputs = nativeBuildInputs ++ [ pkgs.makeWrapper ];
          inherit buildInputs;

          postInstall = ''
            install -Dm644 crates/slopp/assets/linux/org.slopp.Slopp.desktop \
              $out/share/applications/org.slopp.Slopp.desktop
            install -Dm644 crates/slopp/assets/linux/org.slopp.Slopp.svg \
              $out/share/icons/hicolor/scalable/apps/org.slopp.Slopp.svg
          '';

          postFixup = ''
            wrapProgram $out/bin/slopp \
              --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath runtimeLibs} \
              --suffix PATH : ${pkgs.lib.makeBinPath modules}
            wrapProgram $out/bin/sloppctl \
              --suffix PATH : ${pkgs.lib.makeBinPath modules}
          '';

          meta = {
            description = "XAMPP-style control panel built with GPUI";
            mainProgram = "slopp";
            platforms = pkgs.lib.platforms.linux;
          };
        };
      in
      {
        packages.default = slopp;

        apps.default = {
          type = "app";
          program = "${slopp}/bin/slopp";
        };

        devShells.default = pkgs.mkShell {
          inherit buildInputs;
          nativeBuildInputs =
            nativeBuildInputs
            ++ modules
            ++ (with pkgs; [
              rustc
              cargo
              clippy
              rustfmt
              rust-analyzer
            ]);
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs;
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };
      }
    );
}
