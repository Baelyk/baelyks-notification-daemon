{
  description = "Rust";

  inputs = {
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    fenix,
    nixpkgs,
    flake-utils,
  }: let
    name = "baelyks-notification-daemon";
    displayname = "Baelyk's notification daemon";
  in
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = nixpkgs.legacyPackages.${system};
      toolchain = fenix.packages.${system}.stable.toolchain;

      # For Iced, https://github.com/iced-rs/iced/blob/master/DEPENDENCIES.md
      dlopenLibraries = with pkgs; [
        libxkbcommon
        vulkan-loader
        wayland
      ];
      rpath = nixpkgs.lib.makeLibraryPath dlopenLibraries;
    in {
      packages.default =
        (pkgs.makeRustPlatform {
          cargo = toolchain;
          rustc = toolchain;
        })
        .buildRustPackage {
          pname = name;
          version = "0.2.0";

          src = ./.;

          cargoLock.lockFile = ./Cargo.lock;
          cargoLock.outputHashes = {
            "cryoglyph-0.1.0" = "sha256-X7S9jq8wU6g1DDNEzOtP3lKWugDnpopPDBK49iWvD4o=";
            "dpi-0.1.1" = "sha256-hlVhlQ8MmIbNFNr6BM4edKdZbe+ixnPpKm819zauFLQ=";
            "iced-0.14.0-dev" = "sha256-LXRUGHrhop2qh+DtTN0ZlnntIvJACRPpgVCyglvodEs=";
            "iced_exdevtools-0.14.0-dev" = "sha256-5vDaLIq8c8/e4MbemUO8esMJpeIR4AyssKUQLQffyWA=";
          };

          # For Iced, modified based on Halloy's nixpkg
          buildInputs = dlopenLibraries;
          postFixup = ''
            rpath=$(patchelf --print-rpath $out/bin/${name})
            patchelf --set-rpath "$rpath:${nixpkgs.lib.makeLibraryPath dlopenLibraries}" $out/bin/${name}
          '';

          # DBUS Service file
          postInstall = ''
            mkdir -p $out/share/dbus-1/services
            cat <<END > $out/share/dbus-1/services/org.baelyk.${name}.service
            [D-BUS Service]
            Name=org.freedesktop.Notifications
            Exec=$out/bin/${name}
            SystemdService=${name}.service
          '';
        };

      devShells.default = pkgs.mkShell {
        packages = [
          toolchain
        ];

        # For Iced, https://github.com/iced-rs/iced/blob/master/DEPENDENCIES.md
        env.RUSTFLAGS = "-C link-arg=-Wl,-rpath,${rpath}";

        shellHook = ''
          echo $(cargo --version)

          exec fish
        '';
      };
    })
    // flake-utils.lib.eachDefaultSystemPassThrough (system: {
      nixosModules.default = {
        config,
        lib,
        ...
      }: let
        cfg = config.services.${name};
      in {
        options = {
          services.${name} = {
            enable = lib.mkEnableOption displayname;

            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${system}.default;
              defaultText = lib.literalExpression "self.pacakges.default";
              description = "Package providing {command}`${name}`.";
            };
          };
        };

        config = lib.mkIf cfg.enable {
          home.packages = [cfg.package];

          systemd.user.services.${name} = {
            Unit = {
              Description = displayname;
              #After = [ "graphical-sessions.pre.target" ];
              #PartOf = [ "graphical-session.target" ];
            };

            Service = {
              Type = "dbus";
              BusName = "org.freedesktop.Notifications";
              ExecStart = "${cfg.package}/bin/${name}";
            };
          };
        };
      };
    });
}
