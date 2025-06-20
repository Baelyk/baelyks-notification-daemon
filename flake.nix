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

      devShells.default = pkgs.mkShell rec {
        # For Iced https://github.com/iced-rs/iced/blob/master/DEPENDENCIES.md
        buildInputs = with pkgs; [
          expat
          fontconfig
          freetype
          freetype.dev
          libGL
          pkg-config
          xorg.libX11
          xorg.libXcursor
          xorg.libXi
          xorg.libXrandr
          wayland
          libxkbcommon
        ];
        LD_LIBRARY_PATH = builtins.foldl' (a: b: "${a}:${b}/lib") "${pkgs.vulkan-loader}/lib" buildInputs;

        packages = [
          toolchain
        ];

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
