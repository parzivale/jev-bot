{ self }:
{ config, lib, pkgs, ... }:

let
  cfg = config.services.jev-bot;
in
{
  options.services.jev-bot = {
    enable = lib.mkEnableOption "the jev-bot Discord bot";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.jev-bot;
      defaultText = lib.literalExpression "jev-bot";
      description = "The jev-bot package to run.";
    };

    tokenFile = lib.mkOption {
      type = lib.types.path;
      example = "/run/secrets/jev-bot-discord-token";
      description = ''
        Path to a file containing the Discord bot token, and nothing else.

        Read at runtime through a systemd credential, so the token never
        enters the Nix store or the unit's environment. A trailing newline
        is fine.
      '';
    };

    apiKeyFile = lib.mkOption {
      type = lib.types.path;
      example = "/run/secrets/jev-bot-typesafe-key";
      description = ''
        Path to a file containing the TypeSafe API key, and nothing else.
        Handled exactly like {option}`services.jev-bot.tokenFile`.
      '';
    };

    guildId = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "123456789012345678";
      description = ''
        Register commands in this guild, where they appear immediately.
        When null they register globally, which takes up to an hour to
        propagate.
      '';
    };

    model = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "jev-latest";
      description = "Override the TypeSafe model. Null uses the bot's default.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.services.jev-bot = {
      description = "jev-bot Discord bot";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];

      # %d is the systemd credentials directory, readable only by this unit.
      environment = {
        DISCORD_TOKEN_FILE = "%d/discord-token";
        TYPESAFE_API_KEY_FILE = "%d/typesafe-api-key";
        SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
      }
      // lib.optionalAttrs (cfg.guildId != null) { DISCORD_GUILD_ID = cfg.guildId; }
      // lib.optionalAttrs (cfg.model != null) { TYPESAFE_MODEL = cfg.model; };

      serviceConfig = {
        ExecStart = lib.getExe cfg.package;
        LoadCredential = [
          "discord-token:${cfg.tokenFile}"
          "typesafe-api-key:${cfg.apiKeyFile}"
        ];

        Restart = "on-failure";
        RestartSec = 5;

        DynamicUser = true;
        CapabilityBoundingSet = "";
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        NoNewPrivileges = true;
        PrivateDevices = true;
        PrivateTmp = true;
        ProtectClock = true;
        ProtectControlGroups = true;
        ProtectHome = true;
        ProtectHostname = true;
        ProtectKernelLogs = true;
        ProtectKernelModules = true;
        ProtectKernelTunables = true;
        ProtectSystem = "strict";
        RestrictAddressFamilies = [ "AF_INET" "AF_INET6" ];
        RestrictNamespaces = true;
        RestrictRealtime = true;
        RestrictSUIDSGID = true;
        SystemCallArchitectures = "native";
        SystemCallFilter = [ "@system-service" "~@privileged" "~@resources" ];
      };
    };
  };
}
